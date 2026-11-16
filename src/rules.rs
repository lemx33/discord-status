//! Rules: window matching, away messages, and turning a rule into a
//! Discord activity payload. Ported from the Python original's rule engine.

use anyhow::{anyhow, Result};
use regex::{Regex, RegexBuilder};
use serde_json::{json, Map, Value};

pub const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
pub const TEXT_FIELDS: [&str; 4] = ["details", "state", "large_text", "small_text"];
pub const IMAGE_FIELDS: [&str; 2] = ["large_image", "small_image"];

/// Local time, the only parts of "now" the rules care about.
#[derive(Clone, Copy, Debug)]
pub struct LocalTime {
    /// Day of week, 0 = Monday .. 6 = Sunday (matches tm_wday).
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// A rule with its matchers precompiled. `raw` keeps the original JSON keys so
/// unknown settings (buttons, large_text, ...) survive a round trip.
#[derive(Clone, Debug)]
pub struct Rule {
    pub raw: Map<String, Value>,
    pub procs: Vec<String>,
    pub title: Option<Regex>,
}

pub fn compile_rules(rules: &[Value]) -> Result<Vec<Rule>> {
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        let obj = rule
            .as_object()
            .cloned()
            .unwrap_or_default();
        let procs = obj
            .get("process")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|p| p.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let title = match obj.get("title").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => Some(
                RegexBuilder::new(t)
                    .case_insensitive(true)
                    .build()
                    .map_err(|e| anyhow!(e.to_string()))?,
            ),
            _ => None,
        };
        out.push(Rule { raw: obj, procs, title });
    }
    Ok(out)
}

pub struct MatchOut<'a> {
    pub index: usize,
    pub rule: &'a Rule,
    pub groups: Map<String, Value>,
}

/// First rule that fits the foreground window, like the Python original.
pub fn match_rule<'a>(rules: &'a [Rule], exe: &str, title: &str) -> Option<MatchOut<'a>> {
    for (i, rule) in rules.iter().enumerate() {
        if !rule.procs.is_empty() && !rule.procs.contains(&exe.to_lowercase()) {
            continue;
        }
        let mut groups: Map<String, Value> = Map::new();
        if let Some(re) = &rule.title {
            let caps = match re.captures(title) {
                Some(c) => c,
                None => continue,
            };
            for name in re.capture_names().flatten() {
                let v = caps
                    .name(name)
                    .map(|m| m.as_str().trim())
                    .unwrap_or("");
                groups.insert(name.to_string(), Value::String(v.to_string()));
            }
        }
        return Some(MatchOut { index: i, rule, groups });
    }
    None
}

/// Fill `{name}` placeholders in a template with the regex groups. Missing
/// groups become empty text, like the Python original's blank-substitution dict.
pub fn fill_template(template: &str, groups: &Map<String, Value>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut it = template.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => {
                if it.peek() == Some(&'{') {
                    it.next();
                    out.push('{');
                    continue;
                }
                let mut name = String::new();
                let mut closed = false;
                for nc in it.by_ref() {
                    if nc == '}' {
                        closed = true;
                        break;
                    }
                    name.push(nc);
                }
                if closed {
                    if let Some(v) = groups.get(&name) {
                        if let Some(s) = v.as_str() {
                            out.push_str(s);
                        }
                    }
                } else {
                    out.push('{');
                    out.push_str(&name);
                }
            }
            '}' => {
                if it.peek() == Some(&'}') {
                    it.next();
                    out.push('}');
                } else {
                    out.push('}');
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Which line the member list shows next to your name.
pub fn status_display_type(value: &str) -> Option<u8> {
    match value {
        "name" => Some(0),
        "state" => Some(1),
        "details" => Some(2),
        _ => None,
    }
}

/// Turn a rule into a Discord activity JSON object (None means clear it).
pub fn build_activity(
    rule: &Map<String, Value>,
    groups: &Map<String, Value>,
    started: f64,
    status_display: Option<&str>,
) -> Option<Value> {
    let mut act = Map::new();
    let mut text = Map::new();
    for field in TEXT_FIELDS {
        if let Some(template) = rule.get(field).and_then(Value::as_str) {
            if template.is_empty() {
                continue;
            }
            let value = fill_template(template, groups).trim().to_string();
            if value.chars().count() >= 2 {
                let truncated: String = value.chars().take(128).collect();
                text.insert(field.to_string(), Value::String(truncated));
            }
        }
    }
    for field in ["details", "state"] {
        if let Some(v) = text.get(field) {
            act.insert(field.to_string(), v.clone());
        }
    }
    let mut assets = Map::new();
    for field in IMAGE_FIELDS {
        if let Some(v) = rule.get(field) {
            if !v.is_null() {
                assets.insert(field.to_string(), v.clone());
            }
        }
    }
    for field in ["large_text", "small_text"] {
        if let Some(v) = text.get(field) {
            let image_key = field.replace("text", "image");
            if assets.contains_key(&image_key) {
                assets.insert(field.to_string(), v.clone());
            }
        }
    }
    if !assets.is_empty() {
        act.insert("assets".to_string(), Value::Object(assets));
    }
    if let Some(t) = rule.get("type").and_then(Value::as_i64) {
        if matches!(t, 0 | 2 | 3 | 5) {
            act.insert("type".to_string(), json!(t));
        }
    }
    let display_raw = rule
        .get("status_display")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| status_display.map(|s| s.to_string()));
    if let Some(raw) = display_raw {
        if let Some(d) = status_display_type(&raw) {
            if d != 0 {
                act.insert("status_display_type".to_string(), json!(d));
            }
        }
    }
    let show_elapsed = rule
        .get("show_elapsed")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if show_elapsed {
        act.insert("timestamps".to_string(), json!({ "start": started as i64 }));
    }
    if let Some(buttons) = rule.get("buttons").and_then(Value::as_array) {
        if !buttons.is_empty() {
            let kept: Vec<Value> = buttons.iter().take(2).cloned().collect();
            act.insert("buttons".to_string(), Value::Array(kept));
        }
    }
    if act.is_empty() {
        None
    } else {
        Some(Value::Object(act))
    }
}

// ---------------------------------------------------------------------------
// Away messages
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct IdleRule {
    pub raw: Map<String, Value>,
    pub from: u32,
    pub to: u32,
    pub days: Vec<u32>,
}

/// Parse a time of day like "23:00", "7", "7:30am" or "11 PM" into minutes.
pub fn clock_minutes(text: &str) -> Result<u32> {
    // (\d{1,2})(?:[:.](\d{2}))?\s*(?:([ap])\.?\s*m\.?)?\s*   (case-insensitive)
    let re = Regex::new(r"(?i)^\s*(\d{1,2})(?:[:.](\d{2}))?\s*(?:([ap])\.?\s*m\.?)?\s*$")
        .expect("clock regex");
    let caps = match re.captures(text) {
        Some(c) => c,
        None => return Err(anyhow!("{:?} isn't a time (try something like 23:00 or 11pm)", text)),
    };
    let hour: u32 = caps.get(1).unwrap().as_str().parse().unwrap();
    let minute: u32 = caps
        .get(2)
        .map(|m| m.as_str().parse().unwrap())
        .unwrap_or(0);
    if let Some(ap) = caps.get(3) {
        if !(1..=12).contains(&hour) {
            return Err(anyhow!("{:?} isn't a time", text));
        }
        let hour = hour % 12 + if ap.as_str().eq_ignore_ascii_case("p") { 12 } else { 0 };
        return Ok(hour * 60 + minute);
    }
    if hour > 23 || minute > 59 {
        return Err(anyhow!("{:?} isn't a time", text));
    }
    Ok(hour * 60 + minute)
}

/// "mon", "Tuesday", "fri" -> 0..6. Matches the Python tm_wday numbering.
pub fn day_number(name: &str) -> Result<u32> {
    let key: String = name.trim().to_lowercase().chars().take(3).collect();
    DAY_NAMES
        .iter()
        .position(|d| *d == key)
        .map(|i| i as u32)
        .ok_or_else(|| anyhow!("{:?} isn't a day (use mon, tue, wed, thu, fri, sat or sun)", name))
}

pub fn compile_idle_rules(rules: &[Value]) -> Result<Vec<IdleRule>> {
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        let raw = rule
            .as_object()
            .cloned()
            .unwrap_or_default();
        let from = match raw.get("from").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => clock_minutes(t)?,
            _ => 0,
        };
        let to = match raw.get("to").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => clock_minutes(t)?,
            _ => 24 * 60,
        };
        let mut days = Vec::new();
        if let Some(list) = raw.get("days").and_then(Value::as_array) {
            for d in list {
                if let Some(s) = d.as_str() {
                    days.push(day_number(s)?);
                }
            }
        }
        out.push(IdleRule { raw, from, to, days });
    }
    Ok(out)
}

/// Time window and days. For a window past midnight (Fri 20:00 to 08:00), the
/// early-morning part belongs to the day it started: Sat 02:00 counts as Friday.
fn fits_time(rule: &IdleRule, now: LocalTime) -> bool {
    let (start, end) = (rule.from, rule.to);
    let clock = now.hour * 60 + now.minute;
    let mut day = now.weekday;
    let in_window = if start == end {
        true
    } else if start < end {
        start <= clock && clock < end
    } else {
        let past_midnight = clock < end;
        if past_midnight {
            day = if day == 0 { 6 } else { day - 1 };
        }
        clock >= start || past_midnight
    };
    in_window && (rule.days.is_empty() || rule.days.contains(&day))
}

fn away_float(rule: &Map<String, Value>, key: &str) -> f64 {
    rule.get(key)
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// The away status: the first idle_rules entry that fits the day, time and how
/// long you've been away, else the plain `idle` status. Returns a tag for the
/// "which status is showing" bookkeeping plus the rule itself.
pub enum AwayPick<'a> {
    Rule(usize, &'a IdleRule),
    Plain(Option<&'a Map<String, Value>>),
}

pub fn match_idle<'a>(
    idle_rules: &'a [IdleRule],
    idle_secs: f64,
    idle_status: Option<&'a Map<String, Value>>,
    now: LocalTime,
) -> AwayPick<'a> {
    let away = idle_secs / 60.0;
    for (i, rule) in idle_rules.iter().enumerate() {
        if !fits_time(rule, now) {
            continue;
        }
        if away < away_float(&rule.raw, "min_minutes") {
            continue;
        }
        if let Some(max) = rule.raw.get("max_minutes").and_then(Value::as_f64) {
            if max != 0.0 && away >= max {
                continue;
            }
        }
        return AwayPick::Rule(i, rule);
    }
    AwayPick::Plain(idle_status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(json: serde_json::Value) -> Rule {
        compile_rules(&[json]).unwrap().remove(0)
    }

    #[test]
    fn simple_process_match() {
        let r = rule(json!({"process": ["Code.exe"], "details": "coding"}));
        let rules = vec![r];
        let m = match_rule(&rules, "code.EXE", "main.rs - code").unwrap();
        assert_eq!(m.rule.raw["details"], "coding");
        assert!(match_rule(&rules, "chrome.exe", "main.rs - code").is_none());
    }

    #[test]
    fn regex_groups_and_stripping() {
        let r = rule(json!({"process": ["Code.exe"],
            "title": "^(?P<file>.+?) - (?P<folder>.+?) - Visual Studio Code",
            "state": "editing {file}", "details": "coding in {folder}"}));
        let rules = vec![r];
        let m = match_rule(&rules, "Code.exe", "main.rs - demo - Visual Studio Code").unwrap();
        assert_eq!(m.groups["file"], "main.rs");
        assert_eq!(m.groups["folder"], "demo");
    }

    #[test]
    fn no_match_returns_none() {
        let r = rule(json!({"title": "Spotify"}));
        let rules = vec![r];
        assert!(match_rule(&rules, "code.exe", "main.rs").is_none());
    }

    #[test]
    fn unknown_group_is_blank() {
        let mut groups = Map::new();
        assert_eq!(fill_template("editing {nope}", &groups), "editing ");
        groups.insert("file".into(), json!("a.rs"));
        assert_eq!(fill_template("editing {file}", &groups), "editing a.rs");
        assert_eq!(fill_template("{{x}}", &groups), "{x}");
    }

    #[test]
    fn activity_from_rule() {
        let r = rule(json!({"details": "coding in {folder}", "state": "editing {file}",
            "large_image": "vsc", "large_text": "VS Code", "type": 2,
            "show_elapsed": true}));
        let mut groups = Map::new();
        groups.insert("folder".into(), json!("demo"));
        groups.insert("file".into(), json!("a.rs"));
        let act = build_activity(&r.raw, &groups, 1000.5, Some("details")).unwrap();
        assert_eq!(act["details"], "coding in demo");
        assert_eq!(act["state"], "editing a.rs");
        assert_eq!(act["assets"]["large_text"], "VS Code");
        assert_eq!(act["type"], 2);
        assert_eq!(act["timestamps"]["start"], 1000);
        assert_eq!(act["status_display_type"], 2);
    }

    #[test]
    fn short_text_and_no_type() {
        let r = rule(json!({"details": "x", "state": "ok", "type": 1}));
        let act = build_activity(&r.raw, &Map::new(), 0.0, None).unwrap();
        assert!(act.get("details").is_none()); // 1-char text is dropped
        assert_eq!(act["state"], "ok");
        assert!(act.get("type").is_none()); // type 1 is not a valid Discord type
        assert!(act["timestamps"]["start"] == 0); // show_elapsed defaults to true
        let no_timer = build_activity(
            &json!({"details": "ok", "show_elapsed": false}).as_object().unwrap().clone(),
            &Map::new(),
            5.0,
            None,
        )
        .unwrap();
        assert!(no_timer.get("timestamps").is_none());
    }

    #[test]
    fn clock_parsing() {
        assert_eq!(clock_minutes("23:00").unwrap(), 23 * 60);
        assert_eq!(clock_minutes("7").unwrap(), 7 * 60);
        assert_eq!(clock_minutes("7:30am").unwrap(), 7 * 60 + 30);
        assert_eq!(clock_minutes("11 PM").unwrap(), 23 * 60);
        assert_eq!(clock_minutes("12am").unwrap(), 0);
        assert_eq!(clock_minutes("12:00 pm").unwrap(), 12 * 60);
        assert!(clock_minutes("24:00").is_err());
        assert!(clock_minutes("nope").is_err());
        assert!(clock_minutes("13pm").is_err());
    }

    #[test]
    fn day_parsing() {
        assert_eq!(day_number("mon").unwrap(), 0);
        assert_eq!(day_number("Wednesday").unwrap(), 2);
        assert_eq!(day_number("SUN").unwrap(), 6);
        assert!(day_number("blursday").is_err());
    }

    #[test]
    fn idle_rule_time_windows() {
        // Saturday 02:00 counts as Friday night: the early-morning part of a
        // window past midnight belongs to the day it started.
        let now = LocalTime { weekday: 5, hour: 2, minute: 0 };
        let ir = compile_idle_rules(&[json!({"from": "23:00", "to": "07:00", "days": ["fri"],
            "details": "sleeping"})])
        .unwrap();
        assert!(fits_time(&ir[0], now), "Saturday 2am counts as Friday night");

        let saturday_night = LocalTime { weekday: 5, hour: 23, minute: 30 };
        assert!(!fits_time(&ir[0], saturday_night), "Saturday night is not Friday");

        let now2 = LocalTime { weekday: 5, hour: 10, minute: 0 }; // Saturday 10:00
        assert!(!fits_time(&ir[0], now2));

        let ir2 =
            compile_idle_rules(&[json!({"from": "09:00", "to": "12:00", "details": "class"})])
                .unwrap();
        let in_class = LocalTime { weekday: 1, hour: 10, minute: 0 };
        assert!(fits_time(&ir2[0], in_class));
        let out_class = LocalTime { weekday: 1, hour: 13, minute: 0 };
        assert!(!fits_time(&ir2[0], out_class));
    }

    #[test]
    fn idle_match_minutes() {
        let ir = compile_idle_rules(&[json!({"min_minutes": 60, "details": "gone for a while"})])
            .unwrap();
        let idle = json!({"details": "afk"});
        let idle_obj = idle.as_object().unwrap();
        let now = LocalTime { weekday: 2, hour: 12, minute: 0 };
        let short = match_idle(&ir, 30.0 * 60.0, Some(idle_obj), now);
        let long = match_idle(&ir, 90.0 * 60.0, Some(idle_obj), now);
        match (short, long) {
            (AwayPick::Plain(p), AwayPick::Rule(0, r)) => {
                assert_eq!(p.unwrap()["details"], "afk");
                assert_eq!(r.raw["details"], "gone for a while");
            }
            _ => panic!("wrong idle picks"),
        }
    }
}