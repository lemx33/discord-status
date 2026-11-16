//! Discord Rich Presence that follows whatever app is in the foreground.
//!
//! Windows only. Talks to Discord's local IPC pipe directly (no account token,
//! no selfbot) and reads the foreground window through the Win32 API. Rules
//! live in config.json (created from config.example.json on first run) and are
//! reloaded automatically when the file changes.
//!
//! Usage:
//!     DiscordStatus.exe              run (tray icon + hidden background work)
//!     DiscordStatus.exe --dry-run    print what would be shown, don't touch Discord
//!     DiscordStatus.exe --autostart  start quietly (no setup or control windows)
//!
//! Settings live in %APPDATA%\DiscordStatus.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod config;
mod discord;
mod presence;
mod rules;
mod toast;
mod tray;
mod ui;
mod win;

use anyhow::Result;
use tray::TrayCmd;

fn main() {
    if let Err(e) = real_main() {
        app::log(&format!("fatal: {:#}", e));
        std::process::exit(1);
    }
}

/// The placeholder counts as "no ID yet", like the Python original.
fn usable_client_id(cfg: &config::LoadedConfig) -> String {
    let id = cfg.client_id();
    if id == app::PLACEHOLDER_ID {
        String::new()
    } else {
        id
    }
}

fn real_main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let quiet = args.iter().any(|a| a == "--autostart");
    win::set_dpi_awareness();
    app::ensure_config()?;

    if !dry_run {
        match win::claim_instance() {
            None => {
                // Another copy is already running.
                if quiet {
                    app::log("another copy is already running, exiting");
                } else {
                    // A tiny control window instead of stacking copies.
                    ui::run_control_panel();
                }
                return Ok(());
            }
            Some(lock) => {
                // Keep the mutex alive for the process lifetime.
                std::mem::forget(lock);
            }
        }
        win::reset_stop_event();
        let cfg = match config::LoadedConfig::load(&app::paths().config) {
            Ok(c) => c,
            Err(e) => {
                app::log(&format!("config.json has a mistake: {}", e));
                if !quiet {
                    ui::show_error(format!("{}", e));
                }
                return Ok(());
            }
        };
        if usable_client_id(&cfg).is_empty() {
            if quiet {
                app::log("no Application ID yet: open DiscordStatus.exe once to set it up");
                return Ok(());
            }
            match ui::run_setup() {
                None => {
                    app::log("setup cancelled");
                    return Ok(());
                }
                Some(_) => {}
            }
        }
    }

    let mut toast = toast::Toast::spawn();
    let announce = !quiet && !dry_run;

    if dry_run {
        presence::presence_loop(true, Some(&toast), false, None)?;
        toast.stop();
        return Ok(());
    }

    // Tray icon (and the quiet way to stop the app from it).
    let mut tray_opt = match tray::Tray::spawn(Some(app::paths().icon.clone())) {
        Ok(t) => Some(t),
        Err(e) => {
            app::log(&format!("tray icon disabled: {}", e));
            None
        }
    };
    let (tray_events, tray_tx) = match tray_opt.as_mut() {
        Some(t) => (t.take_events(), Some(t.sender())),
        None => (None, None),
    };

    // The background instance's window: hidden until the tray menu needs it.
    let (ui_handle, ui_tx) = ui::spawn_service(tray_events);

    // Tooltip updates, called whenever the status text changes.
    let on_change = {
        let tx = tray_tx.clone();
        move |details: String, state: String| {
            if let Some(tx) = tx.as_ref() {
                let now = if state.trim().is_empty() {
                    details.clone()
                } else {
                    format!("{} \u{00b7} {}", details, state)
                };
                let _ = tx.send(TrayCmd::SetTooltip(format!("Discord Status\n{}", now)));
                let _ = tx.send(TrayCmd::SetNow(now));
            }
        }
    };

    let result = presence::presence_loop(false, Some(&toast), announce, Some(&on_change));

    // Stopping: tell the window to close, then stop the icon and the popups.
    let _ = ui_tx.send(ui::UiCmd::Quit);
    let _ = ui_handle.join();
    if let Some(mut t) = tray_opt {
        t.stop();
    }
    toast.stop();
    result
}