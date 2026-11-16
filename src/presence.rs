//! The presence loop: watch the foreground window, pick a rule, build an
//! activity, push it over the Discord pipe. Ported from status.py.

use crate::app;
use crate::config::LoadedConfig;
use crate::discord::{DiscordIpc, IpcError};
use crate::rules::{build_activity, match_idle, match_rule, AwayPick};
use crate::toast::Toast;
use crate::win;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

/// The last status we talked to Discord about, so we only send real changes.
/// `Never` doubles as "the status was left exactly as it was".
enum Current {
    Never,
    Sent(Option<Value>),
}

impl Current {
    fn differs_from(&self, activity: &Option<Value>) -> bool {
        match self {
            Current::Sent(cur) => cur != activity,
            Current::Never => true,
        }
    }
}

fn epoch_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn jtext(activity: Option<&Value>) -> String {
    activity
        .map(|a| serde_json::to_string(a).unwrap_or_default())
        .unwrap_or_else(|| "status cleared".into())
}

pub fn presence_loop(
    dry_run: bool,
    toast: Option<&Toast>,
    announce: bool,
    on_change: Option<&dyn Fn(String, String)>,
) -> anyhow::Result<()> {
    let config_path = app::paths().config.clone();
    let mut loaded = LoadedConfig::load(&config_path)?;
    let mut cfg_mtime = std::fs::metadata(&config_path).and_then(|m| m.modified()).ok();
    let mut last_toast: Option<(String, String)> = None;
    if let (Some(toast), true) = (toast, announce) {
        toast.show(
            "Discord Status is running",
            "settings are in the tray icon",
            3.0,
            loaded
                .data
                .get("notify_position")
                .and_then(Value::as_str)
                .unwrap_or("bottom-right"),
        );
    }
    let mut ipc: Option<DiscordIpc> = None;
    let mut next_connect = Instant::now();
    let mut current_key: Option<String> = None;
    let mut current = Current::Never;
    let mut started_at = epoch_now();
    let mut last_sent_at = 0.0;
    let own_pid = std::process::id();

    app::log(if dry_run { "started (dry run)" } else { "started" });
    loop {
        // Hot-reload config.json.
        let mtime = std::fs::metadata(&config_path).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime != cfg_mtime {
            let old_id = loaded.client_id();
            match LoadedConfig::load(&config_path) {
                Ok(new) => {
                    loaded = new;
                    current = Current::Never; // force a resend
                    app::log("config reloaded");
                    if ipc.is_some() && loaded.client_id() != old_id {
                        ipc = None; // new Application ID: reconnect with it
                        next_connect = Instant::now();
                    }
                }
                Err(e) => app::log(&format!("config error, keeping old config: {}", e)),
            }
            cfg_mtime = mtime;
        }

        let idle_after =
            loaded.data.get("idle_minutes").and_then(Value::as_f64).unwrap_or(0.0) * 60.0;
        let idle_secs = win::idle_seconds();
        let (key, rule, groups): (
            Option<String>,
            Option<Map<String, Value>>,
            Map<String, Value>,
        ) = if idle_after > 0.0 && idle_secs >= idle_after {
            let now = win::local_time();
            let idle = loaded.data.get("idle").and_then(Value::as_object).cloned();
            match match_idle(&loaded.idle_rules, idle_secs, idle.as_ref(), now) {
                AwayPick::Rule(i, r) => (Some(format!("idle{}", i)), Some(r.raw.clone()), Map::new()),
                AwayPick::Plain(p) => (Some("idle".to_string()), p.cloned(), Map::new()),
            }
        } else {
            let fg = win::foreground();
            if fg.pid == own_pid {
                // Our own popup somehow got focus: ignore it entirely.
                (
                    current_key.clone(),
                    Some(json!({"keep_previous": true}).as_object().unwrap().clone()),
                    Map::new(),
                )
            } else if let Some(m) = match_rule(&loaded.rules, &fg.exe, &fg.title) {
                (Some(format!("rule{}", m.index)), Some(m.rule.raw.clone()), m.groups)
            } else {
                (
                    Some("default".into()),
                    loaded.data.get("default").and_then(Value::as_object).cloned(),
                    Map::new(),
                )
            }
        };

        let keep_previous = rule
            .as_ref()
            .and_then(|r| r.get("keep_previous"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if keep_previous {
            // e.g. alt-tabbing into Discord itself, or a private window: leave the status alone.
            let poll_ms = (loaded.poll_seconds() * 1000.0) as u32;
            if win::wait_for_stop(poll_ms.max(50)) {
                break;
            }
            continue;
        }

        let away = key.as_deref().map(|k| k.starts_with("idle")).unwrap_or(false);
        if key != current_key {
            // Away statuses count from when you actually left, so switching from
            // "afk" to "gone for a while" doesn't restart the elapsed timer.
            started_at = epoch_now() - if away { idle_secs } else { 0.0 };
            current_key = key.clone();
        }
        let status_display = loaded
            .data
            .get("status_display")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let activity: Option<Value> = match &rule {
            Some(r) => build_activity(r, &groups, started_at, status_display.as_deref()),
            None => None, // a null "default"/"idle" clears the status
        };

        if current.differs_from(&activity) {
            if dry_run {
                app::log(&format!("would show: {}", jtext(activity.as_ref())));
                current = Current::Sent(activity);
            } else if epoch_now() - last_sent_at >= 4.0 {
                // Discord: max 5 updates / 20s.
                let client_id = loaded.client_id();
                if client_id.is_empty() {
                    if Instant::now() >= next_connect {
                        app::log("set client_id in config.json first (see README)");
                        next_connect = Instant::now() + Duration::from_secs(60);
                    }
                } else if ipc.is_none() && Instant::now() >= next_connect {
                    match DiscordIpc::connect(&client_id) {
                        Ok(new_ipc) => {
                            ipc = Some(new_ipc);
                            app::log("connected to Discord");
                        }
                        Err(e) => {
                            app::log(&format!("can't connect: {}", e));
                            ipc = None;
                            next_connect = Instant::now() + Duration::from_secs(15);
                        }
                    }
                }
                if let Some(conn) = ipc.as_mut() {
                    match conn.set_activity(activity.as_ref()) {
                        Ok(()) => {
                            app::log(&format!("showing: {}", jtext(activity.as_ref())));
                            current = Current::Sent(activity.clone());
                            last_sent_at = epoch_now();
                            // Popup only when the visible text changes (not on
                            // reconnects, config reloads or timer resets).
                            let details = activity
                                .as_ref()
                                .and_then(|p| p.get("details"))
                                .and_then(Value::as_str)
                                .unwrap_or("status cleared")
                                .to_string();
                            let state = activity
                                .as_ref()
                                .and_then(|p| p.get("state"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let text = (details, state);
                            if last_toast.as_ref() != Some(&text) {
                                if let Some(toast) = toast {
                                    if loaded
                                        .data
                                        .get("notify")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(true)
                                    {
                                        let seconds = loaded
                                            .data
                                            .get("notify_seconds")
                                            .and_then(Value::as_f64)
                                            .unwrap_or(3.0);
                                        let position = loaded
                                            .data
                                            .get("notify_position")
                                            .and_then(Value::as_str)
                                            .unwrap_or("bottom-right");
                                        toast.show(&text.0, &text.1, seconds, position);
                                    }
                                }
                                if let Some(f) = on_change {
                                    f(text.0.clone(), text.1.clone());
                                }
                                last_toast = Some(text);
                            }
                        }
                        Err(IpcError::BadActivity(msg)) => {
                            // Bad activity payload; the connection itself is still fine.
                            app::log(&msg);
                            current = Current::Sent(activity); // don't spam a bad rule
                        }
                        Err(e) => {
                            app::log(&format!("lost Discord: {}", e));
                            ipc = None;
                            next_connect = Instant::now() + Duration::from_secs(15);
                        }
                    }
                }
            }
        }

        let poll_ms = (loaded.poll_seconds() * 1000.0) as u32;
        if win::wait_for_stop(poll_ms.max(50)) {
            break;
        }
    }

    drop(ipc); // Discord clears the activity when the pipe closes
    app::log("stopped");
    Ok(())
}