//! Describing rules in plain words for the editor's lists. Ported from ui.py.

use crate::rules::{clock_minutes, day_number};
use serde_json::{Map, Value};

const REGEX_CHARS: [char; 13] = ['.', '^', '$', '*', '+', '?', '{', '}', '[', ']', '|', '(', ')'];

/// Like re.escape, but leaves spaces and punctuation readable in the file.
pub fn plain_to_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if REGEX_CHARS.contains(&c) || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The literal text a regex matches, or None if it uses regex features.
pub fn regex_to_plain(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            if i + 1 < chars.len() && !chars[i + 1].is_alphanumeric() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            return None;
        }
        if REGEX_CHARS.contains(&c) {
            return None;
        }
        out.push(c);
        i += 1;
    }
    Some(out)
}

const BROWSERS: [&str; 5] = ["chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "brave.exe"];
const KNOWN_BROWSERS: [&str; 7] = [
    "vivaldi.exe", "librewolf.exe", "zen.exe", "arc.exe", "waterfox.exe", "floorp.exe",
    "iexplore.exe",
];

fn app_names(procs: &[Value]) -> String {
    let names: Vec<String> = procs
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.to_string())
        .collect();
    if names.len() >= 3 && names.iter().all(|p| {
        let lower = p.to_lowercase();
        BROWSERS.contains(&lower.as_str()) || KNOWN_BROWSERS.contains(&lower.as_str())
    }) {
        return "a web browser".into();
    }
    let stripped: Vec<String> = names
        .iter()
        .map(|name| name.strip_suffix(".exe").unwrap_or(name).to_string())
        .collect();
    let extra = if stripped.len() > 3 {
        format!(" +{} more", stripped.len() - 3)
    } else {
        String::new()
    };
    stripped.iter().take(3).cloned().collect::<Vec<_>>().join(", ") + &extra
}

fn describe_title(pattern: &str) -> String {
    if let Some(plain) = regex_to_plain(pattern) {
        return format!("title has \"{}\"", plain);
    }
    if let Some(rest) = pattern.strip_prefix('^') {
        if let Some(plain) = regex_to_plain(rest) {
            return format!("title starts with \"{}\"", plain);
        }
    }
    let words: Vec<Option<String>> = pattern.split('|').map(regex_to_plain).collect();
    if words.len() > 1 && words.iter().all(|w| w.is_some()) {
        let quoted: Vec<String> = words
            .iter()
            .map(|w| format!("\"{}\"", w.as_ref().unwrap()))
            .collect();
        return format!("title has {}", quoted.join(" or "));
    }
    format!("title matches  {}", pattern)
}

pub fn describe_when(rule: &Map<String, Value>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(procs) = rule.get("process").and_then(Value::as_array) {
        if !procs.is_empty() {
            parts.push(app_names(procs));
        }
    }
    if let Some(title) = rule.get("title").and_then(Value::as_str) {
        if !title.is_empty() {
            parts.push(describe_title(title));
        }
    }
    if parts.is_empty() {
        "any window".into()
    } else {
        parts.join("   +   ")
    }
}

/// Minutes after midnight as "11 PM" or "7:30 AM".
pub fn format_clock(minutes: u32) -> String {
    let minutes = minutes % (24 * 60);
    let (hour, minute) = (minutes / 60, minutes % 60);
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    if minute == 0 {
        format!("{} {}", hour12, suffix)
    } else {
        format!("{}:{:02} {}", hour12, minute, suffix)
    }
}

fn format_minutes(n: f64) -> String {
    if n >= 60.0 && n % 60.0 == 0.0 {
        format!("{} h", (n / 60.0) as i64)
    } else {
        format!("{} min", n)
    }
}

pub fn describe_days(days: &[Value]) -> String {
    let mut nums: Vec<u32> = Vec::new();
    for d in days {
        match d.as_str() {
            Some(s) => match day_number(s) {
                Ok(n) => nums.push(n),
                Err(_) => {
                    // A day we can't parse: show it as-is.
                    return days
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ");
                }
            },
            None => {}
        }
    }
    nums.sort_unstable();
    nums.dedup();
    if nums == [0, 1, 2, 3, 4] {
        return "weekdays".into();
    }
    if nums == [5, 6] {
        return "weekends".into();
    }
    let short = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    // runs of consecutive days: Mon–Wed
    let mut groups: Vec<[u32; 2]> = Vec::new();
    for n in &nums {
        if let Some(last) = groups.last_mut() {
            if *n == last[1] + 1 {
                last[1] = *n;
                continue;
            }
        }
        groups.push([*n, *n]);
    }
    let mut out: Vec<String> = Vec::new();
    for [a, b] in groups {
        if b - a >= 2 {
            out.push(format!("{}–{}", short[a as usize], short[b as usize]));
        } else {
            for n in a..=b {
                out.push(short[n as usize].to_string());
            }
        }
    }
    out.join(", ")
}

pub fn describe_away_when(rule: &Map<String, Value>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(days) = rule.get("days").and_then(Value::as_array) {
        if !days.is_empty() {
            parts.push(describe_days(days));
        }
    }
    let has_from = rule.get("from").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false);
    let has_to = rule.get("to").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false);
    if has_from || has_to {
        match (
            rule.get("from").and_then(Value::as_str).filter(|s| !s.is_empty()).map(clock_minutes).transpose(),
            rule.get("to").and_then(Value::as_str).filter(|s| !s.is_empty()).map(clock_minutes).transpose(),
        ) {
            (Ok(start), Ok(end)) => {
                let start = start.unwrap_or(0);
                let end = end.unwrap_or(0);
                parts.push(format!("{} – {}", format_clock(start), format_clock(end)));
            }
            _ => {
                let from = rule.get("from").and_then(Value::as_str).unwrap_or("?");
                let to = rule.get("to").and_then(Value::as_str).unwrap_or("?");
                parts.push(format!("{} – {}", from, to));
            }
        }
    }
    let low = rule.get("min_minutes").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let high = rule.get("max_minutes").and_then(Value::as_f64).filter(|v| *v > 0.0);
    match (low, high) {
        (Some(lo), Some(hi)) => parts.push(format!("away {} to {}", format_minutes(lo), format_minutes(hi))),
        (Some(lo), None) => parts.push(format!("away {} or more", format_minutes(lo))),
        (None, Some(hi)) => parts.push(format!("away less than {}", format_minutes(hi))),
        (None, None) => {}
    }
    if parts.is_empty() {
        "any time".into()
    } else {
        parts.join(",   ")
    }
}

const ACTIVITY_TYPES: [(i64, &str); 4] = [(0, "Playing"), (2, "Listening to"), (3, "Watching"), (5, "Competing in")];

pub fn describe_shows(rule: &Map<String, Value>) -> String {
    if rule.get("keep_previous").and_then(Value::as_bool) == Some(true) {
        return "(leaves your status alone)".into();
    }
    let lines: Vec<&str> = [rule.get("details"), rule.get("state")]
        .iter()
        .filter_map(|v| v.and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .collect();
    let mut text = lines.join("  ·  ");
    let kind = rule.get("type").and_then(Value::as_i64).unwrap_or(0);
    let verb = ACTIVITY_TYPES
        .iter()
        .find(|(n, _)| *n == kind)
        .map(|(_, name)| *name)
        .unwrap_or("Playing");
    if verb != "Playing" {
        text = format!("{}:  {}", verb, text);
    }
    if text.is_empty() {
        "(no text)".into()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_regex_roundtrip() {
        assert_eq!(plain_to_regex("youtube.com"), "youtube\\.com");
        assert_eq!(regex_to_plain("youtube\\.com").unwrap(), "youtube.com");
        assert_eq!(regex_to_plain("^(?P<x>.+)").is_none(), true);
        assert_eq!(regex_to_plain("InPrivate").unwrap(), "InPrivate");
    }

    #[test]
    fn describing() {
        let rule: Map<String, Value> = json!({"process": ["Code.exe"], "title": "Visual Studio Code", "details": "coding"})
            .as_object().unwrap().clone();
        assert_eq!(describe_when(&rule), "Code   +   title has \"Visual Studio Code\"");
        assert_eq!(describe_shows(&rule), "coding");
        let browsers: Map<String, Value> = json!({"process": ["chrome.exe", "msedge.exe", "firefox.exe"], "details": "browsing the web"})
            .as_object().unwrap().clone();
        assert_eq!(describe_when(&browsers), "a web browser");
    }

    #[test]
    fn describing_away() {
        let rule: Map<String, Value> = json!({"from": "23:00", "to": "07:00", "days": ["fri"], "min_minutes": 60, "details": "gone"})
            .as_object().unwrap().clone();
        assert_eq!(describe_away_when(&rule), "Fri,   11 PM – 7 AM,   away 1 h or more");
        assert_eq!(format_clock(23 * 60), "11 PM");
        assert_eq!(format_clock(7 * 60 + 30), "7:30 AM");
        assert_eq!(format_minutes(120.0), "2 h");
        assert_eq!(format_minutes(45.0), "45 min");
    }

    #[test]
    fn days_grouping() {
        assert_eq!(describe_days(&[json!("mon"), json!("tue"), json!("wed")]), "Mon–Wed");
        assert_eq!(describe_days(&[json!("sat"), json!("sun")]), "weekends");
        assert_eq!(describe_days(&[json!("mon"), json!("wed"), json!("fri")]), "Mon, Wed, Fri");
        assert_eq!(describe_days(&[json!("mon"), json!("thu"), json!("sat")]), "Mon, Thu, Sat");
    }
}