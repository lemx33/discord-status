//! Dialogs: editing one rule, editing one away message, picking an open
//! window, and the message / confirm boxes. Ported from ui.py's dialogs.

use super::{describe, font_id, BRIGHT, ERROR_FG, HOVER, LABEL, SIDEBAR, TEXT};
use crate::rules::{clock_minutes, day_number, DAY_NAMES};
use serde_json::{json, Map, Value};

pub const ACTIVITY_TYPES: [(i64, &str); 4] = [
    (0, "Playing"),
    (2, "Listening to"),
    (3, "Watching"),
    (5, "Competing in"),
];
const BROWSERS: [&str; 5] = ["chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "brave.exe"];
const POSITIONS: [&str; 4] = ["bottom-left", "bottom-right", "top-left", "top-right"];
const SHORT_DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const DISPLAYS: [(&str, &str); 3] = [("details", "Line 1"), ("state", "Line 2"), ("name", "The app name")];
pub const DISPLAYS_FOR_EDITOR: [(&str, &str); 3] = DISPLAYS;
pub const POSITIONS_FOR_EDITOR: [&str; 4] = POSITIONS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMode {
    New { insert_at: usize },
    Edit { index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwayMode {
    New { insert_at: usize },
    Edit { index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickPurpose {
    RuleField,
    TestWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DeleteRule(usize),
    DeleteAway(usize),
    CloseEditor { save: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogKind {
    Rule { mode: RuleMode },
    Away { mode: AwayMode },
    Pick { purpose: PickPurpose },
    Confirm { action: ConfirmAction },
    Info,
}

pub enum DialogResult {
    Rule(Value),
    Away(Value),
    Pick(String, String),
    Ok,
    Yes,
    No,
    Cancel,
}

pub enum Dialog {
    Rule(RuleDlg),
    Away(AwayDlg),
    Pick(PickDlg),
    Info(InfoDlg),
    Confirm(ConfirmDlg),
}

impl Dialog {
    pub fn info(title: &str, message: &str) -> Dialog {
        Dialog::Info(InfoDlg {
            title: title.into(),
            message: message.into(),
            result: None,
        })
    }

    pub fn confirm(title: &str, message: &str) -> Dialog {
        Dialog::Confirm(ConfirmDlg {
            title: title.into(),
            message: message.into(),
            action: ConfirmAction::CloseEditor { save: true },
            result: None,
        })
    }

    pub fn confirm_action(title: &str, message: &str, action: ConfirmAction) -> Dialog {
        Dialog::Confirm(ConfirmDlg {
            title: title.into(),
            message: message.into(),
            action,
            result: None,
        })
    }

    pub fn rule(mode: RuleMode, original: Value) -> Dialog {
        Dialog::Rule(RuleDlg::new(mode, original))
    }

    pub fn away(mode: AwayMode, original: Value) -> Dialog {
        Dialog::Away(AwayDlg::new(mode, original))
    }

    pub fn pick(purpose: PickPurpose) -> Dialog {
        Dialog::Pick(PickDlg::new(purpose))
    }

    pub fn kind(&self) -> DialogKind {
        match self {
            Dialog::Rule(r) => DialogKind::Rule { mode: r.mode },
            Dialog::Away(a) => DialogKind::Away { mode: a.mode },
            Dialog::Pick(p) => DialogKind::Pick { purpose: p.purpose },
            Dialog::Confirm(c) => DialogKind::Confirm { action: c.action.clone() },
            Dialog::Info(_) => DialogKind::Info,
        }
    }

    pub fn take_result(&mut self) -> Option<DialogResult> {
        match self {
            Dialog::Rule(r) => {
                if r.cancelled {
                    r.cancelled = false;
                    return Some(DialogResult::Cancel);
                }
                r.result.take().map(DialogResult::Rule)
            }
            Dialog::Away(a) => {
                if a.cancelled {
                    a.cancelled = false;
                    return Some(DialogResult::Cancel);
                }
                a.result.take().map(DialogResult::Away)
            }
            Dialog::Pick(p) => {
                if p.cancelled {
                    p.cancelled = false;
                    return Some(DialogResult::Cancel);
                }
                p.result
                    .take()
                    .map(|(exe, title)| DialogResult::Pick(exe, title))
            }
            Dialog::Info(i) => i.result.take().map(|_| DialogResult::Ok),
            Dialog::Confirm(c) => c.result.take().map(|r| match r {
                ConfirmResult::Yes => DialogResult::Yes,
                ConfirmResult::No => DialogResult::No,
                ConfirmResult::Cancel => DialogResult::Cancel,
            }),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        match self {
            Dialog::Rule(r) => r.show(ctx),
            Dialog::Away(a) => a.show(ctx),
            Dialog::Pick(p) => p.show(ctx),
            Dialog::Info(i) => i.show(ctx),
            Dialog::Confirm(c) => c.show(ctx),
        }
    }
}

enum ConfirmResult {
    Yes,
    No,
    Cancel,
}

pub struct InfoDlg {
    title: String,
    message: String,
    result: Option<()>,
}

impl InfoDlg {
    fn show(&mut self, ctx: &egui::Context) {
        let title = self.title.clone();
        let message = self.message.clone();
        let open = egui::Window::new(&title)
            .collapsible(false)
            .resizable(false)
            .default_size([420.0, 180.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(egui::RichText::new(message).color(TEXT).font(font_id(12.0)));
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::primary_button(ui, "OK").clicked() {
                        self.result = Some(());
                    }
                });
            });
        let _ = open;
    }
}

pub struct ConfirmDlg {
    title: String,
    message: String,
    action: ConfirmAction,
    result: Option<ConfirmResult>,
}

impl ConfirmDlg {
    fn show(&mut self, ctx: &egui::Context) {
        let title = self.title.clone();
        let message = self.message.clone();
        egui::Window::new(&title)
            .collapsible(false)
            .resizable(false)
            .default_size([420.0, 170.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(egui::RichText::new(&message).color(TEXT).font(font_id(12.0)));
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::primary_button(ui, "Yes").clicked() {
                        self.result = Some(ConfirmResult::Yes);
                    }
                    if super::button(ui, "No").clicked() {
                        self.result = Some(ConfirmResult::No);
                    }
                    if super::button(ui, "Cancel").clicked() {
                        self.result = Some(ConfirmResult::Cancel);
                    }
                });
            });
    }
}

// ---------------------------------------------------------------------------
// Pick an open window
// ---------------------------------------------------------------------------

pub struct PickDlg {
    purpose: PickPurpose,
    windows: Vec<(String, String)>,
    selected: Option<usize>,
    result: Option<(String, String)>,
    cancelled: bool,
}

impl PickDlg {
    fn new(purpose: PickPurpose) -> PickDlg {
        let mut windows = crate::win::list_windows();
        windows.sort_by(|a, b| {
            (a.0.to_lowercase(), a.1.to_lowercase()).cmp(&(b.0.to_lowercase(), b.1.to_lowercase()))
        });
        PickDlg { purpose, windows, selected: None, result: None, cancelled: false }
    }

    fn refresh(&mut self) {
        let mut windows = crate::win::list_windows();
        windows.sort_by(|a, b| {
            (a.0.to_lowercase(), a.1.to_lowercase()).cmp(&(b.0.to_lowercase(), b.1.to_lowercase()))
        });
        self.windows = windows;
        self.selected = None;
    }

    fn show(&mut self, ctx: &egui::Context) {
        egui::Window::new("Pick a window")
            .collapsible(false)
            .resizable(true)
            .default_size([560.0, 420.0])
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Pick the app you want this rule for. If it isn't listed, open it, then press Refresh.",
                    )
                    .color(TEXT)
                    .font(font_id(11.0))
                    ,
                );
                ui.add_space(8.0);
                let mut clicked: Option<usize> = None;
                let mut use_selected = false;
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (i, (exe, title)) in self.windows.iter().enumerate() {
                            let label = format!("{}   —   {}", exe, title);
                            let selected = self.selected == Some(i);
                            let (rect, resp) = ui.allocate_at_least(
                                egui::vec2(ui.available_width(), 18.0),
                                egui::Sense::click(),
                            );
                            if resp.clicked() {
                                clicked = Some(i);
                            }
                            if resp.double_clicked() {
                                use_selected = true;
                            }
                            if selected {
                                ui.painter().rect_filled(rect, 0.0, HOVER);
                            }
                            ui.painter().text(
                                rect.left_center() + egui::vec2(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                label,
                                font_id(12.0),
                                if selected { BRIGHT } else { LABEL },
                            );
                        }
                    });
                if let Some(i) = clicked {
                    self.selected = Some(i);
                }
                if use_selected {
                    if let Some(i) = self.selected {
                        self.result = Some((self.windows[i].0.clone(), self.windows[i].1.clone()));
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if super::button(ui, "Refresh").clicked() {
                        self.refresh();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::primary_button(ui, "Use this app").clicked() {
                            if let Some(i) = self.selected {
                                self.result =
                                    Some((self.windows[i].0.clone(), self.windows[i].1.clone()));
                            }
                        }
                        if super::button(ui, "Cancel").clicked() {
                            self.cancelled = true;
                        }
                    });
                });
            });
    }
}

// ---------------------------------------------------------------------------
// Editing one rule
// ---------------------------------------------------------------------------

pub struct RuleDlg {
    mode: RuleMode,
    original: Value,
    apps: String,
    title: String,
    regex: bool,
    seen: String,
    private: bool,
    kind: String,
    details: String,
    state: String,
    image: String,
    elapsed: bool,
    error: String,
    result: Option<Value>,
    /// Set when the user asks to pick an open app; the dialog is opened on top.
    pick_requested: bool,
    done: bool,
    cancelled: bool,
}

impl RuleDlg {
    pub fn new(mode: RuleMode, original: Value) -> RuleDlg {
        let r = original.as_object().cloned().unwrap_or_default();
        let plain = r
            .get("title")
            .and_then(Value::as_str)
            .map(|p| describe::regex_to_plain(p).unwrap_or_else(|| p.to_string()))
            .unwrap_or_default();
        let is_regex = r
            .get("title")
            .and_then(Value::as_str)
            .map(|p| !p.is_empty() && describe::regex_to_plain(p).is_none())
            .unwrap_or(false);
        let kind_name = ACTIVITY_TYPES
            .iter()
            .find(|(n, _)| Some(*n) == r.get("type").and_then(Value::as_i64))
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| "Playing".into());
        RuleDlg {
            mode,
            apps: r
                .get("process")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
            title: plain,
            regex: is_regex,
            seen: String::new(),
            private: r.get("keep_previous").and_then(Value::as_bool).unwrap_or(false),
            kind: kind_name,
            details: r.get("details").and_then(Value::as_str).unwrap_or("").into(),
            state: r.get("state").and_then(Value::as_str).unwrap_or("").into(),
            image: r.get("large_image").and_then(Value::as_str).unwrap_or("").into(),
            elapsed: r.get("show_elapsed").and_then(Value::as_bool).unwrap_or(true),
            error: String::new(),
            result: None,
            pick_requested: false,
            done: false,
            cancelled: false,
            original,
        }
    }

    /// Fill the App / Its-title-now fields from the "pick a window" dialog.
    pub fn set_picked(&mut self, exe: String, title: String) {
        self.apps = exe;
        self.seen = title;
        self.error.clear();
    }

    /// True when the user asked to pick an open app (one-shot flag).
    pub fn take_pick_request(&mut self) -> bool {
        let value = self.pick_requested;
        self.pick_requested = false;
        value
    }

    fn toggle_regex(&mut self) {
        let text = std::mem::take(&mut self.title);
        if self.regex {
            self.title = describe::plain_to_regex(&text);
        } else if let Some(plain) = describe::regex_to_plain(&text) {
            self.title = plain;
        } else {
            self.regex = true;
            self.error = "This pattern uses regular expression features, so it can't be turned back into plain text.".into();
        }
    }

    fn preview(&self) -> String {
        if self.private {
            "Your status stays on whatever it showed before.".into()
        } else {
            let lines: Vec<&str> = [self.details.trim(), self.state.trim()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            lines.join("\n")
        }
    }

    fn ok(&mut self) {
        let mut procs: Vec<String> = Vec::new();
        for name in self.apps.split([',', ';']) {
            let mut name = name.trim().trim_matches('"').to_string();
            if name.is_empty() {
                continue;
            }
            if !name.contains('.') {
                name.push_str(".exe");
            }
            procs.push(name);
        }
        let mut title = self.title.trim().to_string();
        if !title.is_empty() && !self.regex {
            title = describe::plain_to_regex(&title);
        }
        if !title.is_empty() {
            match regex::RegexBuilder::new(&title).case_insensitive(true).build() {
                Ok(_) => {}
                Err(e) => {
                    self.error = format!("The window title pattern has a mistake: {}", e);
                    return;
                }
            }
        }
        let details = self.details.trim().to_string();
        let state = self.state.trim().to_string();
        if !self.private {
            if details.is_empty() && state.is_empty() {
                self.error = "Type what your status should say in Line 1.".into();
                return;
            }
            for (label, text) in [("Line 1", &details), ("Line 2", &state)] {
                if text.chars().count() == 1 {
                    self.error =
                        format!("{} needs at least 2 characters (Discord's rule).", label);
                    return;
                }
            }
        }
        let mut rule = Map::new();
        if !procs.is_empty() {
            rule.insert("process".into(), json!(procs));
        }
        if !title.is_empty() {
            rule.insert("title".into(), json!(title));
        }
        if self.private {
            rule.insert("keep_previous".into(), json!(true));
        } else {
            let kind: Option<i64> = ACTIVITY_TYPES
                .iter()
                .find(|(_, name)| name == &self.kind)
                .map(|(n, _)| *n);
            if let Some(k) = kind {
                if k != 0 {
                    rule.insert("type".into(), json!(k));
                }
            }
            if !details.is_empty() {
                rule.insert("details".into(), json!(details));
            }
            if !state.is_empty() {
                rule.insert("state".into(), json!(state));
            }
            let image = self.image.trim();
            if !image.is_empty() {
                rule.insert("large_image".into(), json!(image));
            }
            if !self.elapsed {
                rule.insert("show_elapsed".into(), json!(false));
            }
        }
        // Keep settings this window doesn't show (buttons, large_text, ...).
        let edited = [
            "process", "title", "keep_previous", "type", "details", "state", "large_image",
            "show_elapsed",
        ];
        if let Some(obj) = self.original.as_object() {
            for (key, value) in obj {
                if !edited.contains(&key.as_str()) {
                    rule.insert(key.clone(), value.clone());
                }
            }
        }
        self.error.clear();
        self.result = Some(Value::Object(rule));
        self.done = true;
    }

    fn show(&mut self, ctx: &egui::Context) {
        if self.done {
            return;
        }
        let head = if self.private {
            "PREVIEW".to_string()
        } else {
            self.kind.to_uppercase()
        };
        let title = match self.mode {
            RuleMode::New { .. } => "New rule",
            RuleMode::Edit { .. } => "Edit rule",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .default_size([520.0, 560.0])
            .show(ctx, |ui| {
                ui.set_width(480.0);
                super::header(ui, "When I'm using…");
                ui.add_space(6.0);
                super::field_label(ui, "App");
                ui.add(
                    egui::TextEdit::singleline(&mut self.apps)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                ui.horizontal(|ui| {
                    if super::button(ui, "Pick an open app…").clicked() {
                        self.pick_requested = true;
                    }
                    if super::button(ui, "Any web browser").clicked() {
                        self.apps = BROWSERS.join(", ");
                    }
                });
                super::hint(
                    ui,
                    "The program's .exe name, like Spotify.exe. Separate several with commas. Leave it empty to match any app.",
                );
                // Shown after picking a window, so the title can be copied from it.
                if !self.seen.is_empty() {
                    super::field_label(ui, "Its title now");
                    ui.add_enabled(
                        false,
                        egui::TextEdit::singleline(&mut self.seen)
                            .desired_width(ui.available_width())
                            .font(font_id(11.0)),
                    );
                }
                ui.add_space(6.0);
                super::field_label(ui, "Window title");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.title)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                if resp.changed() && self.regex {
                    self.error.clear();
                }
                let _ = resp;
                if ui
                    .checkbox(&mut self.regex, "Regular expression (advanced)")
                    .changed()
                {
                    self.toggle_regex();
                }
                super::hint(
                    ui,
                    if self.regex {
                        "Matched with a regular expression, ignoring case. Named groups like (?P<song>.+) can be shown below as {song}."
                    } else {
                        "Only match windows whose title contains this text (any case). Leave empty to match any title."
                    },
                );
                ui.add_space(10.0);
                super::header(ui, "…show this");
                if ui
                    .checkbox(&mut self.private, "Nothing: leave my status as it was (for private stuff)")
                    .changed()
                {
                    self.error.clear();
                }
                let enabled = !self.private;
                super::field_label(ui, "Type");
                let kinds: Vec<String> = ACTIVITY_TYPES.iter().map(|(_, n)| n.to_string()).collect();
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_source("kind_combo")
                        .selected_text(self.kind.clone())
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            for name in &kinds {
                                if ui
                                    .selectable_value(&mut self.kind, name.clone(), name.clone())
                                    .changed()
                                {
                                    self.error.clear();
                                }
                            }
                        });
                });
                super::field_label(ui, "Line 1");
                ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut self.details)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                super::field_label(ui, "Line 2");
                ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut self.state)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                super::field_label(ui, "Picture");
                ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut self.image)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                super::hint(
                    ui,
                    "Optional: an https:// image link, or an Art Asset name from your Discord application.",
                );
                if ui.checkbox(&mut self.elapsed, "Show how long I've been doing it").changed() {
                    self.error.clear();
                }
                // Looks like the activity card on a Discord profile.
                ui.add_space(8.0);
                egui::Frame::group(ui.style())
                    .fill(SIDEBAR)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(&head)
                                .color(LABEL)
                                .font(font_id(8.0))
                                .strong(),
                        );
                        if !self.private {
                            ui.label(
                                egui::RichText::new("Your app name")
                                    .color(BRIGHT)
                                    .font(font_id(10.0))
                                    .strong(),
                            );
                        }
                        ui.label(
                            egui::RichText::new(self.preview())
                                .color(TEXT)
                                .font(font_id(11.0)),
                        );
                    });
                if !self.error.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&self.error).color(ERROR_FG).font(font_id(11.0)));
                }
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::primary_button(ui, "OK").clicked() {
                        self.ok();
                    }
                    if super::button(ui, "Cancel").clicked() {
                        self.cancelled = true;
                    }
                });
            });
    }
}

// ---------------------------------------------------------------------------
// Editing one away message
// ---------------------------------------------------------------------------

pub struct AwayDlg {
    mode: AwayMode,
    original: Value,
    use_days: bool,
    day_vars: [bool; 7],
    use_time: bool,
    start: String,
    end: String,
    use_min: bool,
    low: String,
    use_max: bool,
    high: String,
    kind: String,
    details: String,
    state: String,
    elapsed: bool,
    error: String,
    result: Option<Value>,
    done: bool,
    cancelled: bool,
}

impl AwayDlg {
    pub fn new(mode: AwayMode, original: Value) -> AwayDlg {
        let r = original.as_object().cloned().unwrap_or_default();
        let days: Vec<u32> = r
            .get("days")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|d| day_number(d).ok())
                    .collect()
            })
            .unwrap_or_default();
        let mut day_vars = [false; 7];
        if days.is_empty() {
            for (i, v) in day_vars.iter_mut().enumerate() {
                *v = i < 5;
            }
        } else {
            for d in &days {
                if *d < 7 {
                    day_vars[*d as usize] = true;
                }
            }
        }
        let fmt = |v: Option<&Value>| -> String {
            v.and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(clock_minutes)
                .transpose()
                .ok()
                .flatten()
                .map(super::describe::format_clock)
                .unwrap_or_default()
        };
        let (from, to) = (fmt(r.get("from")), fmt(r.get("to")));
        let kind_name = ACTIVITY_TYPES
            .iter()
            .find(|(n, _)| Some(*n) == r.get("type").and_then(Value::as_i64))
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| "Playing".into());
        AwayDlg {
            mode,
            use_days: !days.is_empty(),
            day_vars,
            use_time: !from.is_empty() || !to.is_empty(),
            start: if from.is_empty() { "11 PM".into() } else { from },
            end: if to.is_empty() { "7 AM".into() } else { to },
            use_min: r.get("min_minutes").and_then(Value::as_f64).is_some(),
            low: r
                .get("min_minutes")
                .and_then(Value::as_f64)
                .map(|n| num(&n))
                .unwrap_or_else(|| "30".into()),
            use_max: r.get("max_minutes").and_then(Value::as_f64).is_some(),
            high: r
                .get("max_minutes")
                .and_then(Value::as_f64)
                .map(|n| num(&n))
                .unwrap_or_else(|| "60".into()),
            kind: kind_name,
            details: r.get("details").and_then(Value::as_str).unwrap_or("").into(),
            state: r.get("state").and_then(Value::as_str).unwrap_or("").into(),
            elapsed: r.get("show_elapsed").and_then(Value::as_bool).unwrap_or(true),
            error: String::new(),
            result: None,
            done: false,
            cancelled: false,
            original,
        }
    }

    fn set_days(&mut self, chosen: &[usize]) {
        for (i, var) in self.day_vars.iter_mut().enumerate() {
            *var = chosen.contains(&i);
        }
    }

    fn ok(&mut self) {
        let mut rule = Map::new();
        let mut when = false;
        if self.use_days {
            let days: Vec<&str> = self
                .day_vars
                .iter()
                .enumerate()
                .filter(|(_, on)| **on)
                .map(|(i, _)| DAY_NAMES[i])
                .collect();
            if days.is_empty() {
                self.error = "Pick at least one day, or untick “it's a”.".into();
                return;
            }
            if days.len() < 7 {
                rule.insert("days".into(), json!(days));
            }
            when = true;
        }
        if self.use_time {
            let start = match clock_minutes(&self.start) {
                Ok(v) => v,
                Err(e) => {
                    self.error = format!("{}.", sentence(&e.to_string()));
                    return;
                }
            };
            let end = match clock_minutes(&self.end) {
                Ok(v) => v,
                Err(e) => {
                    self.error = format!("{}.", sentence(&e.to_string()));
                    return;
                }
            };
            if start == end {
                self.error = "The start and end time are the same.".into();
                return;
            }
            rule.insert("from".into(), json!(format!("{:02}:{:02}", start / 60, start % 60)));
            rule.insert("to".into(), json!(format!("{:02}:{:02}", end / 60, end % 60)));
            when = true;
        }
        for (on, value, key) in [
            (self.use_min, &self.low, "min_minutes"),
            (self.use_max, &self.high, "max_minutes"),
        ] {
            if on {
                let n: f64 = match value.trim().parse() {
                    Ok(v) => v,
                    Err(_) => {
                        self.error = "Minutes have to be a number.".into();
                        return;
                    }
                };
                if n <= 0.0 {
                    self.error = "Minutes have to be more than 0.".into();
                    return;
                }
                rule.insert(key.into(), json!(num_value(n)));
                when = true;
            }
        }
        if let (Some(lo), Some(hi)) = (
            rule.get("min_minutes").and_then(Value::as_f64),
            rule.get("max_minutes").and_then(Value::as_f64),
        ) {
            if lo >= hi {
                self.error = "“At least” has to be smaller than “less than”, or it can never be true.".into();
                return;
            }
        }
        let details = self.details.trim().to_string();
        let state = self.state.trim().to_string();
        if details.is_empty() && state.is_empty() {
            self.error = "Type what your status should say in Line 1.".into();
            return;
        }
        for (label, text) in [("Line 1", &details), ("Line 2", &state)] {
            if text.chars().count() == 1 {
                self.error = format!("{} needs at least 2 characters (Discord's rule).", label);
                return;
            }
        }
        if !when {
            self.error = "Pick at least one thing under “Use this when”, or this message is always used.".into();
            return;
        }
        let kind: Option<i64> = ACTIVITY_TYPES
            .iter()
            .find(|(_, name)| name == &self.kind)
            .map(|(n, _)| *n);
        if let Some(k) = kind {
            if k != 0 {
                rule.insert("type".into(), json!(k));
            }
        }
        if !details.is_empty() {
            rule.insert("details".into(), json!(details));
        }
        if !state.is_empty() {
            rule.insert("state".into(), json!(state));
        }
        if !self.elapsed {
            rule.insert("show_elapsed".into(), json!(false));
        }
        let edited = [
            "days", "from", "to", "min_minutes", "max_minutes", "type", "details", "state",
            "show_elapsed",
        ];
        if let Some(obj) = self.original.as_object() {
            for (key, value) in obj {
                if !edited.contains(&key.as_str()) {
                    rule.insert(key.clone(), value.clone());
                }
            }
        }
        self.error.clear();
        self.result = Some(Value::Object(rule));
        self.done = true;
    }

    fn show(&mut self, ctx: &egui::Context) {
        if self.done {
            return;
        }
        let title = match self.mode {
            AwayMode::New { .. } => "New away message",
            AwayMode::Edit { .. } => "Edit away message",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .default_size([520.0, 580.0])
            .show(ctx, |ui| {
                ui.set_width(480.0);
                super::header(ui, "Use this when…");
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut self.use_days, "it's a").changed() {
                        self.error.clear();
                    }
                    for (i, name) in SHORT_DAYS.iter().enumerate() {
                        if ui.checkbox(&mut self.day_vars[i], *name).changed() {
                            self.error.clear();
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if super::button(ui, "Weekdays").clicked() {
                        self.set_days(&[0, 1, 2, 3, 4]);
                    }
                    if super::button(ui, "Weekends").clicked() {
                        self.set_days(&[5, 6]);
                    }
                    if super::button(ui, "Every day").clicked() {
                        self.set_days(&[0, 1, 2, 3, 4, 5, 6]);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.use_time, "it's between").changed() {
                        self.error.clear();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.start)
                            .desired_width(70.0)
                            .font(font_id(12.0)),
                    );
                    ui.label("and");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.end)
                            .desired_width(70.0)
                            .font(font_id(12.0)),
                    );
                });
                super::hint(
                    ui,
                    "Like 11pm and 7am, or 23:00 and 07:00. It can go past midnight: Friday 11pm to 7am still counts as Friday at 2am.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.use_min, "I've been away at least").changed() {
                        self.error.clear();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.low)
                            .desired_width(50.0)
                            .font(font_id(12.0)),
                    );
                    ui.label("minutes");
                });
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.use_max, "I've been away less than").changed() {
                        self.error.clear();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.high)
                            .desired_width(50.0)
                            .font(font_id(12.0)),
                    );
                    ui.label("minutes");
                });
                super::hint(ui, "Tick more than one and they all have to be true.");
                ui.add_space(10.0);
                super::header(ui, "…show this");
                super::field_label(ui, "Type");
                let kinds: Vec<String> = ACTIVITY_TYPES.iter().map(|(_, n)| n.to_string()).collect();
                egui::ComboBox::from_id_source("away_kind")
                    .selected_text(self.kind.clone())
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for name in &kinds {
                            let clicked = ui
                                .selectable_value(&mut self.kind, name.clone(), name.clone())
                                .changed();
                            let _ = clicked;
                        }
                    });
                super::field_label(ui, "Line 1");
                ui.add(
                    egui::TextEdit::singleline(&mut self.details)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                super::field_label(ui, "Line 2");
                ui.add(
                    egui::TextEdit::singleline(&mut self.state)
                        .desired_width(ui.available_width())
                        .font(font_id(12.0)),
                );
                ui.checkbox(&mut self.elapsed, "Show how long I've been away");
                if !self.error.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&self.error).color(ERROR_FG).font(font_id(11.0)));
                }
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::primary_button(ui, "OK").clicked() {
                        self.ok();
                    }
                    if super::button(ui, "Cancel").clicked() {
                        self.cancelled = true;
                    }
                });
            });
    }
}

fn num(n: &f64) -> String {
    let g = (*n * 100.0).round() / 100.0;
    if g.fract() == 0.0 {
        format!("{}", g as i64)
    } else {
        format!("{}", g)
    }
}

fn num_value(n: f64) -> Value {
    if n.fract() == 0.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

