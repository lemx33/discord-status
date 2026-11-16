//! The rules editor window: the rule list, away messages and general settings,
//! with the unsaved-changes bar. Ported from ui.py's RulesEditor.

use super::dialogs::{AwayMode, DISPLAYS_FOR_EDITOR, POSITIONS_FOR_EDITOR, RuleMode};
use super::{
    describe, font_id, button, field_label, header, hint, primary_button, success_button, BG,
    BORDER, BRIGHT, FIELD, GREY, HINT_FG, LABEL, SELECTED, SIDEBAR, SUCCESS_FG, TEXT,
};
use crate::app;
use crate::config::{read_object, write_config};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

pub enum EditorAction {
    OpenRuleDialog(RuleMode, Value),
    OpenAwayDialog(AwayMode, Value),
    DeleteRule(usize),
    DeleteAway(usize),
    TestWindow,
    OpenNotepad,
    RequestClose,
    Saved,
    SaveError(String),
}

fn obj_of(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn keep_previous(rule: &Value) -> bool {
    rule.as_object()
        .and_then(|r| r.get("keep_previous"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Like Python's "%g": 5 -> "5", 5.5 -> "5.5".
fn gnum(value: Option<&Value>, default: f64) -> String {
    let n = value.and_then(Value::as_f64).unwrap_or(default);
    let rounded = (n * 1_000_000.0).round() / 1_000_000.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{}", rounded)
    }
}

fn text_of(rule: Option<&Value>, key: &str) -> String {
    rule.and_then(Value::as_object)
        .and_then(|r| r.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub struct EditorState {
    cfg: Map<String, Value>,
    pub rules: Vec<Value>,
    pub away: Vec<Value>,
    pub dirty: bool,
    saved_flash: Option<Instant>,
    pub rules_sel: Option<usize>,
    pub away_sel: Option<usize>,
    page: usize,
    // Field values (like the Python StringVars).
    v_client: String,
    v_display: usize,
    v_idle_min: String,
    v_idle1: String,
    v_idle2: String,
    v_default_on: bool,
    v_default1: String,
    v_default2: String,
    v_notify: bool,
    v_notify_secs: String,
    v_position: usize,
    actions: Vec<EditorAction>,
}

impl EditorState {
    pub fn load() -> Result<EditorState> {
        Self::from_cfg(read_object(&app::paths().config)?)
    }

    fn from_cfg(cfg: Map<String, Value>) -> Result<EditorState> {
        let rules = cfg
            .get("rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let away = cfg
            .get("idle_rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // (idle / default are cloned below, once the plain fields are read.)
        let mut client = cfg
            .get("client_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if client == app::PLACEHOLDER_ID {
            client.clear();
        }
        let display_key = cfg
            .get("status_display")
            .and_then(Value::as_str)
            .unwrap_or("details")
            .to_string();
        let position = cfg
            .get("notify_position")
            .and_then(Value::as_str)
            .unwrap_or("bottom-left")
            .to_string();
        let idle_minutes = gnum(cfg_value(&cfg, "idle_minutes"), 5.0);
        let notify_seconds = gnum(cfg_value(&cfg, "notify_seconds"), 2.0);
        let notify = cfg.get("notify").and_then(Value::as_bool).unwrap_or(true);
        // Clone the blocks: `cfg` itself moves into the editor state below.
        let idle = cfg.get("idle").cloned();
        let default = cfg.get("default").cloned();
        Ok(EditorState {
            cfg,
            rules,
            away,
            dirty: false,
            saved_flash: None,
            rules_sel: None,
            away_sel: None,
            page: 0,
            v_client: client,
            v_display: DISPLAYS_FOR_EDITOR
                .iter()
                .position(|(k, _)| *k == display_key)
                .unwrap_or(0),
            v_idle_min: idle_minutes,
            v_idle1: text_of(idle.as_ref(), "details"),
            v_idle2: text_of(idle.as_ref(), "state"),
            v_default_on: !matches!(default, None | Some(Value::Null)),
            v_default1: text_of(default.as_ref(), "details"),
            v_default2: text_of(default.as_ref(), "state"),
            v_notify: notify,
            v_notify_secs: notify_seconds,
            v_position: POSITIONS_FOR_EDITOR
                .iter()
                .position(|p| *p == position)
                .unwrap_or(0),
            actions: Vec::new(),
        })
    }

    pub fn mark_changed(&mut self) {
        self.dirty = true;
        self.saved_flash = None;
    }

    pub fn drain_actions(&mut self) -> Vec<EditorAction> {
        std::mem::take(&mut self.actions)
    }

    /// Save everything: validate, then write config.json atomically.
    pub fn save(&mut self) -> Result<()> {
        let client: String = self.v_client.chars().filter(|c| !c.is_whitespace()).collect();
        if !client.chars().all(|c| c.is_ascii_digit()) || !(17..=20).contains(&client.len()) {
            self.page = 2;
            return Err(anyhow!(
                "The Application ID should be a long number (17–20 digits)."
            ));
        }
        let idle_minutes: f64 = match self.v_idle_min.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                self.page = 1;
                return Err(anyhow!("The away minutes have to be a number."));
            }
        };
        let notify_seconds: f64 = match self.v_notify_secs.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                self.page = 2;
                return Err(anyhow!("The popup seconds have to be a number."));
            }
        };
        if idle_minutes < 0.0 {
            self.page = 1;
            return Err(anyhow!("The away minutes can't be negative."));
        }
        if notify_seconds <= 0.0 {
            self.page = 2;
            return Err(anyhow!("The popup seconds have to be more than 0."));
        }
        for (tab, label, text) in [
            (2, "No-rule Line 1", &self.v_default1),
            (2, "No-rule Line 2", &self.v_default2),
            (1, "Away Line 1", &self.v_idle1),
            (1, "Away Line 2", &self.v_idle2),
        ] {
            if text.trim().chars().count() == 1 {
                self.page = tab;
                return Err(anyhow!(
                    "{} needs at least 2 characters (Discord's rule).",
                    label
                ));
            }
        }

        let mut cfg = self.cfg.clone();
        cfg.insert("client_id".into(), json!(client));
        cfg.insert(
            "status_display".into(),
            json!(DISPLAYS_FOR_EDITOR[self.v_display].0),
        );
        cfg.insert("idle_minutes".into(), float_or_int(idle_minutes));
        cfg.insert("notify".into(), json!(self.v_notify));
        cfg.insert("notify_seconds".into(), float_or_int(notify_seconds));
        cfg.insert(
            "notify_position".into(),
            json!(POSITIONS_FOR_EDITOR[self.v_position]),
        );
        if self.v_default_on {
            cfg.insert(
                "default".into(),
                json!(status_block(self.cfg.get("default"), &self.v_default1, &self.v_default2)),
            );
        } else {
            cfg.insert("default".into(), Value::Null);
        }
        cfg.insert(
            "idle".into(),
            json!(status_block(self.cfg.get("idle"), &self.v_idle1, &self.v_idle2)),
        );
        if !self.away.is_empty() || self.cfg.contains_key("idle_rules") {
            cfg.insert("idle_rules".into(), json!(self.away));
        }
        cfg.insert("rules".into(), json!(self.rules));

        if let Err(e) = write_config(&app::paths().config, &cfg) {
            return Err(anyhow!("Couldn't save: {}", e));
        }
        self.cfg = cfg;
        self.dirty = false;
        self.saved_flash = Some(Instant::now());
        self.actions.push(EditorAction::Saved);
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.cfg = read_object(&app::paths().config)?;
        self.rules = self
            .cfg
            .get("rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.away = self
            .cfg
            .get("idle_rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.rules_sel = None;
        self.away_sel = None;
        let idle = self.cfg.get("idle").cloned();
        let default = self.cfg.get("default").cloned();
        let mut client = self
            .cfg
            .get("client_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if client == app::PLACEHOLDER_ID {
            client.clear();
        }
        self.v_client = client;
        self.v_display = DISPLAYS_FOR_EDITOR
            .iter()
            .position(|(k, _)| {
                *k == self
                    .cfg
                    .get("status_display")
                    .and_then(Value::as_str)
                    .unwrap_or("details")
            })
            .unwrap_or(0);
        self.v_idle_min = gnum(cfg_value(&self.cfg, "idle_minutes"), 5.0);
        self.v_idle1 = text_of(idle.as_ref(), "details");
        self.v_idle2 = text_of(idle.as_ref(), "state");
        self.v_default_on = !matches!(default, None | Some(Value::Null));
        self.v_default1 = text_of(default.as_ref(), "details");
        self.v_default2 = text_of(default.as_ref(), "state");
        self.v_notify = self
            .cfg
            .get("notify")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        self.v_notify_secs = gnum(cfg_value(&self.cfg, "notify_seconds"), 2.0);
        self.v_position = POSITIONS_FOR_EDITOR
            .iter()
            .position(|p| {
                *p == self
                    .cfg
                    .get("notify_position")
                    .and_then(Value::as_str)
                    .unwrap_or("bottom-left")
            })
            .unwrap_or(0);
        self.dirty = false;
        self.saved_flash = None;
        Ok(())
    }

    fn open_rule_dialog(&mut self, index: Option<usize>) {
        match index {
            Some(i) => {
                let original = self.rules[i].clone();
                self.actions
                    .push(EditorAction::OpenRuleDialog(RuleMode::Edit { index: i }, original));
            }
            None => {
                // New rules go below the private "leave my status alone" rules.
                let mut at = 0;
                while at < self.rules.len() && keep_previous(&self.rules[at]) {
                    at += 1;
                }
                self.actions.push(EditorAction::OpenRuleDialog(
                    RuleMode::New { insert_at: at },
                    Value::Null,
                ));
            }
        }
    }

    fn open_away_dialog(&mut self, index: Option<usize>) {
        match index {
            Some(i) => {
                let original = self.away[i].clone();
                self.actions
                    .push(EditorAction::OpenAwayDialog(AwayMode::Edit { index: i }, original));
            }
            None => {
                let at = self.away_sel.unwrap_or(0).min(self.away.len());
                self.actions.push(EditorAction::OpenAwayDialog(
                    AwayMode::New { insert_at: at },
                    Value::Null,
                ));
            }
        }
    }

    pub fn apply_rule(&mut self, mode: RuleMode, rule: Value) {
        match mode {
            RuleMode::New { insert_at } => {
                let at = insert_at.min(self.rules.len());
                self.rules.insert(at, rule);
                self.rules_sel = Some(at);
            }
            RuleMode::Edit { index } => {
                if index < self.rules.len() {
                    self.rules[index] = rule;
                    self.rules_sel = Some(index);
                }
            }
        }
        self.mark_changed();
    }

    pub fn apply_away(&mut self, mode: AwayMode, rule: Value) {
        match mode {
            AwayMode::New { insert_at } => {
                let at = insert_at.min(self.away.len());
                self.away.insert(at, rule);
                self.away_sel = Some(at);
            }
            AwayMode::Edit { index } => {
                if index < self.away.len() {
                    self.away[index] = rule;
                    self.away_sel = Some(index);
                }
            }
        }
        self.mark_changed();
    }

    // -- rendering --------------------------------------------------------

    pub fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // Ctrl+S saves, like the Python editor.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save_and_report();
        }
        let mut sidebar_wants: Option<usize> = None;
        let mut open_notepad = false;
        ui.horizontal_top(|ui| {
            // Sidebar, like Discord's settings.
            ui.vertical(|ui| {
                ui.set_width(200.0);
                egui::Frame::group(ui.style())
                    .fill(SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(14, 20))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("DISCORD STATUS")
                                .color(HINT_FG)
                                .font(font_id(8.0))
                                .strong(),
                        );
                        ui.add_space(8.0);
                        let items = ["Rules", "Away status", "General"];
                        for (i, label) in items.iter().enumerate() {
                            let selected = self.page == i;
                            let (rect, resp) = ui.allocate_at_least(
                                egui::vec2(ui.available_width(), 26.0),
                                egui::Sense::click(),
                            );
                            if resp.clicked() {
                                sidebar_wants = Some(i);
                            }
                            if selected {
                                ui.painter().rect_filled(rect, 4.0, SELECTED);
                            }
                            ui.painter().text(
                                rect.left_center() + egui::vec2(10.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                *label,
                                font_id(12.0),
                                if selected { BRIGHT } else { LABEL },
                            );
                        }
                    });
                ui.add_space(12.0);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Open file in Notepad").color(LABEL),
                        )
                        .fill(SIDEBAR)
                        .frame(false),
                    )
                    .clicked()
                {
                    open_notepad = true;
                }
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                match self.page {
                    0 => self.rules_page(ui),
                    1 => self.away_page(ui),
                    _ => self.general_page(ui),
                }
            });
        });
        if let Some(page) = sidebar_wants {
            self.page = page;
        }
        if open_notepad {
            self.actions.push(EditorAction::OpenNotepad);
        }
        // Bottom bar: quiet normally, "Careful" after an edit, "Saved!" briefly.
        ui.add_space(10.0);
        let flash = self
            .saved_flash
            .map(|t| t.elapsed() < Duration::from_secs(8))
            .unwrap_or(false);
        let (bar_fill, message, message_color) = if self.dirty {
            (FIELD, "Careful — you have unsaved changes!", TEXT)
        } else if flash {
            (BG, "Saved! Your status updates in a few seconds.", SUCCESS_FG)
        } else {
            (BG, "", HINT_FG)
        };
        egui::Frame::group(ui.style())
            .fill(bar_fill)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(message)
                            .color(message_color)
                            .font(font_id(11.0)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.dirty {
                            if success_button(ui, "Save Changes").clicked() {
                                self.save_and_report();
                            }
                            if button(ui, "Reset").clicked() {
                                let _ = self.reset();
                            }
                        } else if button(ui, "Close").clicked() {
                            self.actions.push(EditorAction::RequestClose);
                        }
                    });
                });
            });
    }

    fn save_and_report(&mut self) {
        if let Err(e) = self.save() {
            self.actions.push(EditorAction::SaveError(format!("{}", e)));
        }
    }

    fn rules_page(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(
                "Rules are checked from the top down, and the first one that fits the window you're using sets your status. Put specific rules above general ones.",
            )
            .color(TEXT)
            .font(font_id(11.0))
            ,
        );
        ui.add_space(8.0);
        let mut add = false;
        let mut edit: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let mut test = false;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 170.0).max(200.0));
                let total = ui.available_width();
                let widths = [40.0, total * 0.34, total * 0.42];
                ui.horizontal(|ui| {
                    for text in ["#", "WHEN I'M USING", "MY STATUS SAYS"].iter() {
                        ui.label(
                            egui::RichText::new(*text)
                                .color(HINT_FG)
                                .font(font_id(8.0))
                                .strong(),
                        );
                        ui.add_space(6.0);
                    }
                });
                ui.add_space(2.0);
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (i, rule) in self.rules.iter().enumerate() {
                        let obj = obj_of(rule);
                        let (rect, resp) = ui.allocate_at_least(
                            egui::vec2(total, 22.0),
                            egui::Sense::click(),
                        );
                        let selected = self.rules_sel == Some(i);
                        if resp.clicked() {
                            self.rules_sel = Some(i);
                        }
                        if resp.double_clicked() {
                            edit = Some(i);
                        }
                        if selected {
                            ui.painter().rect_filled(rect, 0.0, SELECTED);
                        }
                        let values = [
                            format!("{}", i + 1),
                            describe::describe_when(&obj),
                            describe::describe_shows(&obj),
                        ];
                        let mut x = rect.left();
                        for (vi, value) in values.iter().enumerate() {
                            ui.painter().text(
                                egui::pos2(x + 6.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                value,
                                font_id(11.0),
                                if selected { BRIGHT } else { LABEL },
                            );
                            x += widths[vi] + 6.0;
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.set_width(150.0);
                if primary_button(ui, "Add…").clicked() {
                    add = true;
                }
                ui.add_space(2.0);
                if ui
                    .add_enabled(self.rules_sel.is_some(), egui::Button::new("Edit…").fill(GREY))
                    .clicked()
                {
                    edit = self.rules_sel;
                }
                if ui
                    .add_enabled(self.rules_sel.is_some(), egui::Button::new("Delete").fill(GREY))
                    .clicked()
                {
                    delete = self.rules_sel;
                }
                ui.add_space(10.0);
                if ui
                    .add_enabled(
                        self.rules_sel.is_some(),
                        egui::Button::new("Move up").fill(GREY),
                    )
                    .clicked()
                {
                    move_up = self.rules_sel;
                }
                if ui
                    .add_enabled(
                        self.rules_sel.is_some(),
                        egui::Button::new("Move down").fill(GREY),
                    )
                    .clicked()
                {
                    move_down = self.rules_sel;
                }
                ui.add_space(10.0);
                if button(ui, "Test a window…").clicked() {
                    test = true;
                }
            });
        });
        hint(
            ui,
            "New rules go above the selected one. “Test a window” shows which rule an app would use.",
        );
        // Apply the button clicks once the borrows above are done.
        if add {
            self.open_rule_dialog(None);
        }
        if let Some(i) = edit {
            self.open_rule_dialog(Some(i));
        }
        if let Some(i) = delete {
            self.actions.push(EditorAction::DeleteRule(i));
        }
        if let Some(i) = move_up {
            self.move_rule(i, -1);
        }
        if let Some(i) = move_down {
            self.move_rule(i, 1);
        }
        if test {
            self.actions.push(EditorAction::TestWindow);
        }
    }

    fn move_rule(&mut self, index: usize, step: isize) {
        if index >= self.rules.len() {
            return;
        }
        let target = index as isize + step;
        if target < 0 || target as usize >= self.rules.len() {
            return;
        }
        self.rules.swap(index, target as usize);
        self.rules_sel = Some(target as usize);
        self.mark_changed();
    }

    fn move_away(&mut self, index: usize, step: isize) {
        if index >= self.away.len() {
            return;
        }
        let target = index as isize + step;
        if target < 0 || target as usize >= self.away.len() {
            return;
        }
        self.away.swap(index, target as usize);
        self.away_sel = Some(target as usize);
        self.mark_changed();
    }

    fn away_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Count me as away after")
                    .color(TEXT)
                    .font(font_id(11.0)),
            );
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.v_idle_min)
                        .desired_width(48.0)
                        .font(font_id(11.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
            ui.label(
                egui::RichText::new("minutes without touching the mouse or keyboard (0 = never)")
                    .color(HINT_FG)
                    .font(font_id(11.0)),
            );
        });
        ui.label(
            egui::RichText::new(
                "Your away status can change with the time of day, or with how long you've been gone. The first message that fits is used.",
            )
            .color(TEXT)
            .font(font_id(11.0))
            ,
        );
        ui.add_space(8.0);
        let mut add = false;
        let mut edit: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 170.0).max(200.0));
                let total = ui.available_width();
                let widths = [40.0, total * 0.42, total * 0.42];
                ui.horizontal(|ui| {
                    for text in ["#", "USE WHEN", "MY STATUS SAYS"].iter() {
                        ui.label(
                            egui::RichText::new(*text)
                                .color(HINT_FG)
                                .font(font_id(8.0))
                                .strong(),
                        );
                        ui.add_space(6.0);
                    }
                });
                ui.add_space(2.0);
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for (i, rule) in self.away.iter().enumerate() {
                        let obj = obj_of(rule);
                        let (rect, resp) =
                            ui.allocate_at_least(egui::vec2(total, 22.0), egui::Sense::click());
                        let selected = self.away_sel == Some(i);
                        if resp.clicked() {
                            self.away_sel = Some(i);
                        }
                        if resp.double_clicked() {
                            edit = Some(i);
                        }
                        if selected {
                            ui.painter().rect_filled(rect, 0.0, SELECTED);
                        }
                        let values = [
                            format!("{}", i + 1),
                            describe::describe_away_when(&obj),
                            describe::describe_shows(&obj),
                        ];
                        let mut x = rect.left();
                        for (vi, value) in values.iter().enumerate() {
                            ui.painter().text(
                                egui::pos2(x + 6.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                value,
                                font_id(11.0),
                                if selected { BRIGHT } else { LABEL },
                            );
                            x += widths[vi] + 6.0;
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.set_width(150.0);
                if primary_button(ui, "Add…").clicked() {
                    add = true;
                }
                ui.add_space(2.0);
                if ui
                    .add_enabled(self.away_sel.is_some(), egui::Button::new("Edit…").fill(GREY))
                    .clicked()
                {
                    edit = self.away_sel;
                }
                if ui
                    .add_enabled(self.away_sel.is_some(), egui::Button::new("Delete").fill(GREY))
                    .clicked()
                {
                    delete = self.away_sel;
                }
                ui.add_space(10.0);
                if ui
                    .add_enabled(
                        self.away_sel.is_some(),
                        egui::Button::new("Move up").fill(GREY),
                    )
                    .clicked()
                {
                    move_up = self.away_sel;
                }
                if ui
                    .add_enabled(
                        self.away_sel.is_some(),
                        egui::Button::new("Move down").fill(GREY),
                    )
                    .clicked()
                {
                    move_down = self.away_sel;
                }
            });
        });
        ui.add_space(12.0);
        header(ui, "If none of those fit");
        ui.horizontal(|ui| {
            field_label(ui, "Line 1");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.v_idle1)
                        .desired_width(320.0)
                        .font(font_id(12.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
        });
        ui.horizontal(|ui| {
            field_label(ui, "Line 2");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.v_idle2)
                        .desired_width(320.0)
                        .font(font_id(12.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
        });
        if add {
            self.open_away_dialog(None);
        }
        if let Some(i) = edit {
            self.open_away_dialog(Some(i));
        }
        if let Some(i) = delete {
            self.actions.push(EditorAction::DeleteAway(i));
        }
        if let Some(i) = move_up {
            self.move_away(i, -1);
        }
        if let Some(i) = move_down {
            self.move_away(i, 1);
        }
    }

    fn general_page(&mut self, ui: &mut egui::Ui) {
        header(ui, "Discord");
        ui.horizontal(|ui| {
            field_label(ui, "Application ID");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.v_client)
                        .desired_width(220.0)
                        .font(font_id(12.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
        });
        ui.horizontal(|ui| {
            field_label(ui, "Member list shows");
            let names: Vec<String> = DISPLAYS_FOR_EDITOR.iter().map(|(_, n)| n.to_string()).collect();
            let changed = egui::ComboBox::from_id_source("v_display")
                .selected_text(names[self.v_display].clone())
                .width(160.0)
                .show_ui(ui, |ui| {
                    let mut any = false;
                    for (i, name) in names.iter().enumerate() {
                        if ui.selectable_value(&mut self.v_display, i, name.clone()).changed() {
                            any = true;
                        }
                    }
                    any
                });
            if changed.inner == Some(true) {
                self.mark_changed();
            }
        });
        ui.add_space(12.0);
        header(ui, "When no rule fits");
        if ui
            .checkbox(&mut self.v_default_on, "Show a status (otherwise it's cleared)")
            .changed()
        {
            self.mark_changed();
        }
        ui.horizontal(|ui| {
            field_label(ui, "Line 1");
            if ui
                .add_enabled(
                    self.v_default_on,
                    egui::TextEdit::singleline(&mut self.v_default1)
                        .desired_width(320.0)
                        .font(font_id(12.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
        });
        ui.horizontal(|ui| {
            field_label(ui, "Line 2");
            if ui
                .add_enabled(
                    self.v_default_on,
                    egui::TextEdit::singleline(&mut self.v_default2)
                        .desired_width(320.0)
                        .font(font_id(12.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
        });
        ui.add_space(12.0);
        header(ui, "Popup");
        if ui
            .checkbox(
                &mut self.v_notify,
                "Show a small popup when my status changes",
            )
            .changed()
        {
            self.mark_changed();
        }
        ui.horizontal(|ui| {
            field_label(ui, "Stays for");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.v_notify_secs)
                        .desired_width(48.0)
                        .font(font_id(11.0)),
                )
                .changed()
            {
                self.mark_changed();
            }
            ui.label(egui::RichText::new("seconds").color(HINT_FG).font(font_id(11.0)));
        });
        ui.horizontal(|ui| {
            field_label(ui, "Corner");
            let changed = egui::ComboBox::from_id_source("v_position")
                .selected_text(POSITIONS_FOR_EDITOR[self.v_position])
                .width(160.0)
                .show_ui(ui, |ui| {
                    let mut any = false;
                    for (i, name) in POSITIONS_FOR_EDITOR.iter().enumerate() {
                        if ui.selectable_value(&mut self.v_position, i, *name).changed() {
                            any = true;
                        }
                    }
                    any
                });
            if changed.inner == Some(true) {
                self.mark_changed();
            }
        });
    }
}

fn cfg_value<'a>(cfg: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    cfg.get(key)
}

fn float_or_int(n: f64) -> Value {
    if n.fract() == 0.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// A status block (like `idle`/`default`): keeps other keys, sets the text.
fn status_block(original: Option<&Value>, details: &str, state: &str) -> Map<String, Value> {
    let mut block = original.and_then(Value::as_object).cloned().unwrap_or_default();
    for (key, value) in [("details", details.trim()), ("state", state.trim())] {
        if value.is_empty() {
            block.remove(key);
        } else {
            block.insert(key.into(), json!(value));
        }
    }
    block
}

