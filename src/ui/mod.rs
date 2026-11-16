//! Small native windows: first-run setup, the control panel and the rules
//! editor. egui stands in for the Tk UI the Python original had; the look and
//! the flow are kept (Discord's dark theme, sidebar editor, bottom save bar).

pub mod describe;
pub mod dialogs;
pub mod editor;

use crate::app;
use crate::tray::TrayEvent;
use crate::win;
use dialogs::{ConfirmAction, Dialog, DialogKind, DialogResult, PickPurpose};
use editor::{EditorAction, EditorState};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

pub const PORTAL_URL: &str = "https://discord.com/developers/applications";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Hidden,
    Setup,
    Control,
    Editor,
}

pub enum UiCmd {
    RunSetup { reply: Sender<Option<(String, bool)>> },
    OpenControlPanel,
    OpenEditor,
    ShowError(String),
    Quit,
}

// Discord's dark theme (from ui.py's palette).
pub const BG: egui::Color32 = egui::Color32::from_rgb(0x31, 0x33, 0x38);
pub const SIDEBAR: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x2d, 0x31);
pub const FIELD: egui::Color32 = egui::Color32::from_rgb(0x1e, 0x1f, 0x22);
pub const HOVER: egui::Color32 = egui::Color32::from_rgb(0x35, 0x37, 0x3c);
pub const SELECTED: egui::Color32 = egui::Color32::from_rgb(0x40, 0x42, 0x49);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0x3f, 0x41, 0x47);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xdb, 0xde, 0xe1);
pub const BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xf2, 0xf3, 0xf5);
pub const LABEL: egui::Color32 = egui::Color32::from_rgb(0xb5, 0xba, 0xc1);
pub const HINT_FG: egui::Color32 = egui::Color32::from_rgb(0x94, 0x9b, 0xa4);
pub const BLURPLE: egui::Color32 = egui::Color32::from_rgb(0x58, 0x65, 0xf2);
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(0x24, 0x80, 0x46);
pub const RED: egui::Color32 = egui::Color32::from_rgb(0xda, 0x37, 0x3c);
pub const GREY: egui::Color32 = egui::Color32::from_rgb(0x4e, 0x50, 0x58);
pub const GREY_HOVER: egui::Color32 = egui::Color32::from_rgb(0x6d, 0x6f, 0x78);
pub const ERROR_FG: egui::Color32 = egui::Color32::from_rgb(0xf2, 0x3f, 0x43);
pub const SUCCESS_FG: egui::Color32 = egui::Color32::from_rgb(0x23, 0xa5, 0x5a);

pub fn font_id(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

pub fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.widgets.noninteractive.bg_fill = BG;
    style.visuals.widgets.inactive.bg_fill = GREY;
    style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::WHITE;
    style.visuals.widgets.hovered.bg_fill = GREY_HOVER;
    style.visuals.widgets.active.bg_fill = GREY_HOVER;
    style.visuals.widgets.open.bg_fill = GREY_HOVER;
    style.visuals.extreme_bg_color = FIELD;
    style.visuals.faint_bg_color = BG;
    style.visuals.code_bg_color = FIELD;
    style.visuals.selection.bg_fill = BLURPLE;
    style.visuals.hyperlink_color = egui::Color32::from_rgb(0x00, 0xa8, 0xfc);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.text_styles.insert(egui::TextStyle::Body, font_id(13.0));
    style.text_styles.insert(egui::TextStyle::Button, font_id(13.0));
    style.text_styles.insert(egui::TextStyle::Heading, font_id(18.0));
    style.text_styles.insert(egui::TextStyle::Small, font_id(11.0));
    ctx.set_style(style);
    // Segoe UI when Windows has it, so the UI matches the system.
    let segoe = std::path::Path::new(r"C:\Windows\Fonts\segoeui.ttf");
    if let Ok(bytes) = std::fs::read(segoe) {
        if !bytes.is_empty() {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("segoe".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.insert(0, "segoe".to_owned());
            }
            ctx.set_fonts(fonts);
        }
    }
}

/// Discord-ish buttons: grey, blurple for the main action, green to save, red to stop.
pub fn colored_button(ui: &mut egui::Ui, text: &str, fill: egui::Color32) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE)).fill(fill))
}

pub fn button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    colored_button(ui, text, GREY)
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    colored_button(ui, text, BLURPLE)
}

pub fn success_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    colored_button(ui, text, GREEN)
}

pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    colored_button(ui, text, RED)
}

/// Section header: small, bold, uppercase.
pub fn header(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .color(LABEL)
            .font(font_id(11.0))
            .strong(),
    );
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_string())
            .color(HINT_FG)
            .font(font_id(11.0)),
    );
}

pub fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(LABEL));
}

fn window_icon() -> Option<egui::IconData> {
    let path = app::paths().icon.clone();
    let img = image::open(&path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

fn native_options() -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([880.0, 580.0])
        .with_min_inner_size([620.0, 420.0]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}

/// -- one-shot windows ---------------------------------------------------

/// First-run window. Returns Some((client id, start with windows)) on Start.
pub fn run_setup() -> Option<(String, bool)> {
    let (tx, rx) = mpsc::channel();
    run_app(UiCmd::RunSetup { reply: tx });
    rx.recv().ok().flatten()
}

/// Shown when the app is opened while it's already running.
pub fn run_control_panel() {
    run_app(UiCmd::OpenControlPanel);
}

pub fn show_error(message: String) {
    run_app(UiCmd::ShowError(message));
}

/// The background instance's window: hidden until the tray menu needs it.
pub fn spawn_service(
    tray_events: Option<Receiver<TrayEvent>>,
) -> (JoinHandle<()>, Sender<UiCmd>) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("ui".into())
        .spawn(move || run_app_with(rx, tray_events))
        .expect("ui thread");
    (handle, tx)
}

fn run_app(cmd: UiCmd) {
    let (tx, rx) = mpsc::channel();
    let _ = tx.send(cmd);
    run_app_with(rx, None);
}

fn run_app_with(rx: Receiver<UiCmd>, tray_events: Option<Receiver<TrayEvent>>) {
    let start_cmd = rx.recv().ok();
    let options = native_options();
    let mut app = UiApp {
        rx,
        tray_events,
        page: Page::Hidden,
        editor: None,
        dialogs: Vec::new(),
        setup: None,
        control: ControlState::default(),
        error_text: None,
        close_requested: false,
        editor_pending_close: false,
        first_frame: true,
    };
    if let Some(cmd) = start_cmd {
        app.handle(cmd);
    }
    let _ = eframe::run_native("Discord Status", options, Box::new(move |cc| {
        apply_theme(&cc.egui_ctx);
        Ok(Box::new(app))
    }));
}

// ---------------------------------------------------------------------------
// The app itself
// ---------------------------------------------------------------------------

pub struct SetupState {
    pub client: String,
    pub autostart: bool,
    pub error: String,
    pub reply: Sender<Option<(String, bool)>>,
}

#[derive(Default)]
pub struct ControlState {
    pub autostart: bool,
}

impl ControlState {
    pub fn fresh() -> Self {
        ControlState { autostart: app::autostart_enabled() }
    }
}

pub struct UiApp {
    rx: Receiver<UiCmd>,
    tray_events: Option<Receiver<TrayEvent>>,
    page: Page,
    editor: Option<EditorState>,
    dialogs: Vec<Dialog>,
    setup: Option<SetupState>,
    control: ControlState,
    error_text: Option<String>,
    close_requested: bool,
    editor_pending_close: bool,
    first_frame: bool,
}

impl UiApp {
    fn handle(&mut self, cmd: UiCmd) {
        match cmd {
            UiCmd::RunSetup { reply } => {
                let cfg = crate::config::Config::load(&app::paths().config).unwrap_or_default();
                self.setup = Some(SetupState {
                    client: app::current_client_id(&cfg),
                    autostart: true,
                    error: String::new(),
                    reply,
                });
                self.page = Page::Setup;
            }
            UiCmd::OpenControlPanel => {
                self.control = ControlState::fresh();
                self.page = Page::Control;
            }
            UiCmd::ShowError(message) => {
                self.error_text = Some(message);
                self.control = ControlState::fresh();
                self.page = Page::Control;
            }
            UiCmd::OpenEditor => {
                self.editor_pending_close = false;
                match EditorState::load() {
                    Ok(editor) => {
                        self.editor = Some(editor);
                        self.page = Page::Editor;
                    }
                    Err(e) => {
                        self.dialogs.push(dialogs::Dialog::info(
                            "Couldn't open your settings",
                            &format!(
                                "{}\n\nFix {} and try again, or open it in Notepad to edit it by hand.",
                                e,
                                app::paths().config.display()
                            ),
                        ));
                    }
                }
            }
            UiCmd::Quit => self.close_requested = true,
        }
    }

    fn close_window_cmd(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn go_hidden(&mut self, ctx: &egui::Context) {
        self.editor = None;
        self.dialogs.clear();
        self.setup = None;
        self.error_text = None;
        self.editor_pending_close = false;
        self.page = Page::Hidden;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn handle_tray(&mut self, ev: TrayEvent) {
        match ev {
            TrayEvent::EditRules => self.handle(UiCmd::OpenEditor),
            TrayEvent::OpenLog => {
                let path = app::paths().log.clone();
                if path.exists() {
                    app::open_in_notepad(&path);
                } else {
                    self.dialogs.push(dialogs::Dialog::info("Open log", "Nothing logged yet."));
                }
            }
            TrayEvent::ToggleAutostart => {
                let enabled = app::autostart_enabled();
                if let Err(e) = app::set_autostart(!enabled) {
                    self.dialogs.push(dialogs::Dialog::info(
                        "Start with Windows",
                        &format!("Couldn't change that setting:\n{}", e),
                    ));
                }
            }
            TrayEvent::Quit => {
                win::request_stop();
                self.handle(UiCmd::Quit);
            }
        }
    }

    fn on_close_requested(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Setup => {
                if let Some(setup) = self.setup.take() {
                    let _ = setup.reply.send(None);
                }
                app::log("setup cancelled");
                Self::close_window_cmd(ctx);
            }
            Page::Control => Self::close_window_cmd(ctx),
            Page::Editor => {
                let dirty = self.editor.as_ref().map(|e| e.dirty).unwrap_or(false);
                if !dirty {
                    self.go_hidden(ctx);
                } else {
                    self.dialogs.push(dialogs::Dialog::confirm(
                        "Save your changes?",
                        "Your rules have unsaved changes.",
                    ));
                    self.editor_pending_close = true;
                }
            }
            Page::Hidden => {}
        }
    }

    fn on_dialog_result(
        &mut self,
        kind: DialogKind,
        result: DialogResult,
        ctx: &egui::Context,
    ) {
        match (kind, result) {
            (DialogKind::Rule { mode }, DialogResult::Rule(rule)) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.apply_rule(mode, rule);
                }
            }
            (DialogKind::Away { mode }, DialogResult::Away(rule)) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.apply_away(mode, rule);
                }
            }
            (DialogKind::Pick { purpose }, DialogResult::Pick(exe, title)) => match purpose {
                PickPurpose::RuleField => {
                    // Hand the picked app back to the rule dialog underneath.
                    if let Some(Dialog::Rule(rd)) = self.dialogs.last_mut() {
                        rd.set_picked(exe, title);
                    }
                }
                PickPurpose::TestWindow => self.test_window(&exe, &title),
            },
            (DialogKind::Confirm { action }, DialogResult::Yes) => match action {
                ConfirmAction::DeleteRule(index) => {
                    if let Some(editor) = self.editor.as_mut() {
                        if index < editor.rules.len() {
                            editor.rules.remove(index);
                            editor.rules_sel = editor
                                .rules_sel
                                .filter(|s| *s < editor.rules.len())
                                .or(Some(index.saturating_sub(1)))
                                .filter(|_| !editor.rules.is_empty());
                            editor.mark_changed();
                        }
                    }
                }
                ConfirmAction::DeleteAway(index) => {
                    if let Some(editor) = self.editor.as_mut() {
                        if index < editor.away.len() {
                            editor.away.remove(index);
                            editor.away_sel = editor
                                .away_sel
                                .filter(|s| *s < editor.away.len())
                                .or(Some(index.saturating_sub(1)))
                                .filter(|_| !editor.away.is_empty());
                            editor.mark_changed();
                        }
                    }
                }
                ConfirmAction::CloseEditor { save } => {
                    if save {
                        if let Some(editor) = self.editor.as_mut() {
                            if editor.dirty {
                                if let Err(e) = editor.save() {
                                    self.dialogs.push(dialogs::Dialog::info(
                                        "Couldn't save",
                                        &format!("{}", e),
                                    ));
                                    return;
                                }
                            }
                        }
                    }
                    self.editor_pending_close = false;
                    self.go_hidden(ctx);
                }
            },
            (DialogKind::Confirm { .. }, DialogResult::No) => {
                if self.editor_pending_close {
                    self.editor_pending_close = false;
                    self.go_hidden(ctx);
                }
            }
            (DialogKind::Confirm { .. }, DialogResult::Cancel) => {
                self.editor_pending_close = false;
            }
            _ => {}
        }
    }

    fn test_window(&mut self, exe: &str, title: &str) {
        let editor = match self.editor.as_ref() {
            Some(e) => e,
            None => return,
        };
        match crate::rules::compile_rules(&editor.rules) {
            Ok(rules) => match crate::rules::match_rule(&rules, exe, title) {
                Some(m) => {
                    let says = if m
                        .rule
                        .raw
                        .get("keep_previous")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    {
                        "leaves your status as it was".to_string()
                    } else {
                        let mut lines: Vec<String> = Vec::new();
                        for f in ["details", "state"] {
                            if let Some(t) =
                                m.rule.raw.get(f).and_then(serde_json::Value::as_str)
                            {
                                lines.push(
                                    crate::rules::fill_template(t, &m.groups).trim().to_string(),
                                );
                            }
                        }
                        format!("shows:\n    {}", lines.join("\n    "))
                    };
                    if let Some(editor) = self.editor.as_mut() {
                        editor.rules_sel = Some(m.index);
                    }
                    self.dialogs.push(dialogs::Dialog::info(
                        "Test a window",
                        &format!("{} — {}\n\nUses rule #{}, which {}", exe, title, m.index + 1, says),
                    ));
                }
                None => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.rules_sel = None;
                    }
                    self.dialogs.push(dialogs::Dialog::info(
                        "Test a window",
                        &format!(
                            "{} — {}\n\nNo rule fits this window, so it shows your \"nothing matches\" status from the General tab.",
                            exe, title
                        ),
                    ));
                }
            },
            Err(e) => {
                self.dialogs.push(dialogs::Dialog::info(
                    "Test a window",
                    &format!("One of the rules has a broken title pattern:\n{}", e),
                ));
            }
        }
    }

    fn setup_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut setup = match self.setup.take() {
            Some(s) => s,
            None => return,
        };
        let mut cancelled = false;
        let mut started: Option<(String, bool)> = None;
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Discord Status")
                    .font(font_id(22.0))
                    .color(BRIGHT)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(
                    "Your Discord status will follow whatever you're doing on your PC.\n\n\
                     1.  Create an application in the Discord Developer Portal. Its name is what your status will say you're playing.\n\
                     2.  Copy its Application ID and paste it below.\n\
                     3.  In Discord, go to User Settings → Activity Privacy and turn on “Share your detected activities with others”.\n\n\
                     Once it's running, click its icon in the system tray (next to the clock) to change your rules.",
                )
                .color(TEXT)
                .font(font_id(12.0)),
            );
            ui.add_space(8.0);
            if colored_button(ui, "Open Developer Portal", FIELD).clicked() {
                app::open_url(PORTAL_URL);
            }
            ui.add_space(14.0);
            field_label(ui, "Application ID");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut setup.client)
                    .desired_width(320.0)
                    .font(font_id(13.0)),
            );
            let enter_pressed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !setup.error.is_empty() {
                ui.label(egui::RichText::new(&setup.error).color(ERROR_FG));
            }
            ui.add_space(8.0);
            ui.checkbox(&mut setup.autostart, "Start with Windows");
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, "Start").clicked() || enter_pressed {
                    let value: String = setup.client.chars().filter(|c| !c.is_whitespace()).collect();
                    if !value.chars().all(|c| c.is_ascii_digit())
                        || !(17..=20).contains(&value.len())
                    {
                        setup.error = "That doesn't look like an Application ID. It's a long number (17–20 digits).".into();
                    } else if let Err(e) = app::save_client_id(&value) {
                        setup.error = format!("Couldn't save settings: {}", e);
                    } else {
                        started = Some((value, setup.autostart));
                    }
                }
                if button(ui, "Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
        if cancelled {
            let _ = setup.reply.send(None);
            app::log("setup cancelled");
            Self::close_window_cmd(ctx);
        } else if let Some((id, autostart)) = started {
            if autostart {
                let _ = app::set_autostart(true);
            } else {
                let _ = app::set_autostart(false);
            }
            app::log("setup finished, starting");
            let _ = setup.reply.send(Some((id, autostart)));
            Self::close_window_cmd(ctx);
        } else {
            self.setup = Some(setup);
        }
    }

    }

impl eframe::App for UiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
        }
        while let Ok(cmd) = self.rx.try_recv() {
            self.handle(cmd);
        }
        // Collect first: handle_tray needs &mut self, which the receiver borrows.
        let tray_events: Vec<TrayEvent> = self
            .tray_events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in tray_events {
            self.handle_tray(ev);
        }

        // Keep polling the channels even when nothing is on screen.
        ctx.request_repaint_after(Duration::from_millis(250));

        if self.close_requested {
            self.close_requested = false;
            Self::close_window_cmd(ctx);
            return;
        }

        // A window close (the X button): we decide what it means per page.
        if self.page != Page::Hidden && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.on_close_requested(ctx);
            return;
        }

        let title = match self.page {
            Page::Setup => "Discord Status setup",
            Page::Control => "Discord Status",
            Page::Editor => "Discord Status rules",
            Page::Hidden => "Discord Status",
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_string()));
        let visible = !matches!(self.page, Page::Hidden) || !self.dialogs.is_empty();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));

        let mut editor_actions: Vec<EditorAction> = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(BG))
            .show(ctx, |ui| match self.page {
                Page::Setup => self.setup_ui(ui, ctx),
                Page::Control => self.control_ui(ui, ctx),
                Page::Editor => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.show(ui, ctx);
                        editor_actions = editor.drain_actions();
                    }
                }
                Page::Hidden => {}
            });
        for action in editor_actions {
            self.handle_editor_action(action, ctx);
        }

        // Dialogs float above everything.
        if !self.dialogs.is_empty() {
            let rect = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("dim"),
            ));
            painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
            // The rule dialog can ask to pick an open app: that opens on top.
            let wants_pick = match self.dialogs.last_mut() {
                Some(Dialog::Rule(rd)) => rd.take_pick_request(),
                _ => false,
            };
            if wants_pick {
                self.dialogs.push(Dialog::pick(PickPurpose::RuleField));
            }
            {
                let top = self.dialogs.last_mut().unwrap();
                top.show(ctx);
            }
            // Pop every dialog that finished (a completed pick un-asks the rule).
            while let Some(mut dialog) = self.dialogs.pop() {
                if let Some(result) = dialog.take_result() {
                    let kind = dialog.kind();
                    self.on_dialog_result(kind, result, ctx);
                } else {
                    self.dialogs.push(dialog);
                    break;
                }
            }
        }
    }
}

impl UiApp {
    fn handle_editor_action(&mut self, action: EditorAction, ctx: &egui::Context) {
        match action {
            EditorAction::OpenRuleDialog(mode, original) => {
                self.dialogs.push(dialogs::Dialog::rule(mode, original));
            }
            EditorAction::OpenAwayDialog(mode, original) => {
                self.dialogs.push(dialogs::Dialog::away(mode, original));
            }
            EditorAction::DeleteRule(index) => {
                if let Some(editor) = self.editor.as_ref() {
                    if index < editor.rules.len() {
                        let rule = &editor.rules[index];
                        let message = format!(
                            "Delete this rule?\n\n{}\n→ {}",
                            crate::ui::describe::describe_when(
                                rule.as_object().unwrap_or(&serde_json::Map::new())
                            ),
                            crate::ui::describe::describe_shows(
                                rule.as_object().unwrap_or(&serde_json::Map::new())
                            ),
                        );
                        self.dialogs.push(dialogs::Dialog::confirm_action(
                            "Delete rule?",
                            &message,
                            ConfirmAction::DeleteRule(index),
                        ));
                    }
                }
            }
            EditorAction::DeleteAway(index) => {
                if let Some(editor) = self.editor.as_ref() {
                    if index < editor.away.len() {
                        let rule = &editor.away[index];
                        let message = format!(
                            "Delete this away message?\n\n{}\n→ {}",
                            crate::ui::describe::describe_away_when(
                                rule.as_object().unwrap_or(&serde_json::Map::new())
                            ),
                            crate::ui::describe::describe_shows(
                                rule.as_object().unwrap_or(&serde_json::Map::new())
                            ),
                        );
                        self.dialogs.push(dialogs::Dialog::confirm_action(
                            "Delete away message?",
                            &message,
                            ConfirmAction::DeleteAway(index),
                        ));
                    }
                }
            }
            EditorAction::TestWindow => {
                self.dialogs.push(dialogs::Dialog::pick(PickPurpose::TestWindow));
            }
            EditorAction::OpenNotepad => {
                if self.editor.as_ref().map(|e| e.dirty).unwrap_or(false) {
                    self.dialogs.push(dialogs::Dialog::confirm(
                        "Save your changes?",
                        "Your rules have unsaved changes.",
                    ));
                    self.editor_pending_close = true;
                } else {
                    app::open_in_notepad(&app::paths().config);
                }
            }
            EditorAction::RequestClose => self.on_close_requested(ctx),
            EditorAction::SaveError(message) => {
                self.dialogs.push(dialogs::Dialog::info("Couldn't save", &message));
            }
            EditorAction::Saved => {}
        }
    }
}

impl UiApp {
    fn control_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Discord Status is running")
                    .font(font_id(22.0))
                    .color(BRIGHT)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(
                    "It's working in the background, with an icon in the system tray (you may need to click ^ next to the clock to see it). Use Edit rules to change what your status says for each app; changes apply as soon as you save.",
                )
                .color(TEXT)
                .font(font_id(12.0)),
            );
            ui.add_space(8.0);
            if ui
                .checkbox(&mut self.control.autostart, "Start with Windows")
                .changed()
            {
                let enabled = self.control.autostart;
                if let Err(e) = app::set_autostart(enabled) {
                    self.control.autostart = app::autostart_enabled();
                    self.dialogs.push(dialogs::Dialog::info(
                        "Start with Windows",
                        &format!("Couldn't change the start with Windows setting:\n{}", e),
                    ));
                }
            }
            if let Some(err) = self.error_text.take() {
                ui.add_space(8.0);
                egui::Frame::group(ui.style())
                    .fill(FIELD)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Your settings file has a mistake in it:")
                                .color(ERROR_FG),
                        );
                        ui.label(egui::RichText::new(&err).color(TEXT).font(font_id(11.0)));
                        ui.add_space(8.0);
                        if colored_button(ui, "Open file in Notepad", FIELD).clicked() {
                            app::open_in_notepad(&app::paths().config);
                        }
                    });
            }
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                if primary_button(ui, "Edit rules").clicked() {
                    self.handle(UiCmd::OpenEditor);
                }
                if button(ui, "Open log").clicked() {
                    let path = app::paths().log.clone();
                    if path.exists() {
                        app::open_in_notepad(&path);
                    } else {
                        self.dialogs.push(dialogs::Dialog::info("Open log", "Nothing logged yet."));
                    }
                }
                if danger_button(ui, "Stop").clicked() {
                    win::request_stop();
                    Self::close_window_cmd(ctx);
                }
            });
        });
    }
}


