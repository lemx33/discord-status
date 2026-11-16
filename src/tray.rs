//! Notification-area (tray) icon with a right-click menu, using plain Win32.
//! The icon lives on its own thread with its own message loop; menu callbacks
//! hand events back to the UI thread over a channel.

use crate::app;
use crate::win;
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use windows::{
    core::{PCWSTR, Result as WinRes},
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
                NIM_MODIFY, NOTIFYICONDATAW, NOTIFY_ICON_MESSAGE,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
                DestroyMenu, DestroyWindow, GetCursorPos, GetMessageW, GetSystemMetrics,
                GetWindowLongPtrW, LoadIconW, LoadImageW, PostMessageW, PostQuitMessage,
                RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu,
                TranslateMessage, DispatchMessageW, GWLP_USERDATA, MSG, WM_APP, WM_CLOSE,
                WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_CONTEXTMENU,
                CREATESTRUCTW, GDI_IMAGE_TYPE, IDI_APPLICATION, IMAGE_FLAGS, IMAGE_ICON,
                LR_LOADFROMFILE, MENU_ITEM_FLAGS, TRACK_POPUP_MENU_FLAGS, WNDCLASSW, WS_OVERLAPPED,
                HICON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                SM_CXSMICON, SM_CYSMICON, WINDOW_EX_STYLE,
            },
        },
    },
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_UPDATE_TIP: u32 = WM_APP + 2;
const WM_APP_CMDS: u32 = WM_APP + 3;

const MF_STRING: u32 = 0x0;
const MF_SEPARATOR: u32 = 0x800;
const MF_CHECKED: u32 = 0x8;
const MF_GRAYED: u32 = 0x1;

const MENU_ID_NOW: usize = 1;
const MENU_ID_EDIT: usize = 3;
const MENU_ID_LOG: usize = 4;
const MENU_ID_AUTOSTART: usize = 5;
const MENU_ID_QUIT: usize = 7;

pub enum TrayCmd {
    SetTooltip(String),
    SetNow(String),
}

pub enum TrayEvent {
    EditRules,
    OpenLog,
    ToggleAutostart,
    Quit,
}

pub struct Tray {
    tx: Sender<TrayCmd>,
    events: Option<Receiver<TrayEvent>>,
    thread: Option<JoinHandle<()>>,
    hwnd: std::sync::Arc<std::sync::Mutex<Option<isize>>>,
}

impl Tray {
    /// Start the icon, or fail if the shell won't take it (Explorer missing).
    pub fn spawn(icon_path: Option<PathBuf>) -> Result<Tray> {
        let (tx, rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let hwnd = std::sync::Arc::new(std::sync::Mutex::new(None));
        let hwnd2 = hwnd.clone();
        // Sent as soon as the icon is in the shell, before the message loop.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let thread = std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                let result = run(rx, ev_tx, hwnd2, icon_path, ready_tx);
                if let Err(e) = result {
                    app::log(&format!("tray icon stopped: {}", e));
                }
            })
            .map_err(|e| anyhow!("can't start tray thread: {}", e))?;
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| anyhow!("tray icon timed out"))??;
        Ok(Tray { tx, events: Some(ev_rx), thread: Some(thread), hwnd })
    }

    /// Take the event channel: the UI thread consumes menu clicks from it.
    pub fn take_events(&mut self) -> Option<Receiver<TrayEvent>> {
        self.events.take()
    }

    /// A sender for tooltip / "Now:" updates from other threads.
    pub fn sender(&self) -> Sender<TrayCmd> {
        self.tx.clone()
    }

    pub fn set_tooltip(&self, text: String) {
        let _ = self.tx.send(TrayCmd::SetTooltip(text));
        self.post(WM_UPDATE_TIP);
    }

    pub fn set_now(&self, text: String) {
        let _ = self.tx.send(TrayCmd::SetNow(text));
        self.post(WM_APP_CMDS);
    }

    pub fn stop(&mut self) {
        self.post(WM_CLOSE);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn post(&self, msg: u32) {
        let hwnd = *self.hwnd.lock().unwrap();
        if let Some(h) = hwnd {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(0), LPARAM(0));
            }
        }
    }
}

struct TrayState {
    hwnd: HWND,
    hicon: Option<HICON>,
    taskbar_created: u32,
    tooltip: String,
    now: String,
    rx: Receiver<TrayCmd>,
    tx_events: Sender<TrayEvent>,
}

impl TrayState {
    fn notify(&self, action: NOTIFY_ICON_MESSAGE) -> bool {
        unsafe {
            let mut data = NOTIFYICONDATAW::default();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.hwnd;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = WM_TRAY;
            data.hIcon = self.hicon.unwrap_or(HICON(std::ptr::null_mut()));
            let tip: Vec<u16> = self.tooltip.encode_utf16().take(127).collect();
            let n = tip.len().min(127);
            data.szTip[..n].copy_from_slice(&tip[..n]);
            Shell_NotifyIconW(action, &data).as_bool()
        }
    }

    fn show_menu(&self) {
        unsafe {
            let now = {
                let n = self.now.chars().count();
                if n <= 60 {
                    self.now.clone()
                } else {
                    self.now.chars().take(59).collect::<String>() + "\u{2026}"
                }
            };
            let checked_start = app::autostart_enabled();
            let items: Vec<(usize, String, u32, bool, bool)> = vec![
                (MENU_ID_NOW, format!("Now: {}", now.replace('&', "&&")), MF_GRAYED, false, false),
                (0, String::new(), MF_SEPARATOR, false, false),
                (MENU_ID_EDIT, "Edit rules\u{2026}".into(), MF_STRING, false, true),
                (MENU_ID_LOG, "Open log".into(), MF_STRING, false, true),
                (MENU_ID_AUTOSTART, "Start with Windows".into(), MF_STRING, checked_start, true),
                (0, String::new(), MF_SEPARATOR, false, false),
                (MENU_ID_QUIT, "Quit".into(), MF_STRING, false, true),
            ];
            let hmenu = match CreatePopupMenu() {
                Ok(m) => m,
                Err(_) => return,
            };
            let mut labels: Vec<Vec<u16>> = Vec::new();
            for (_i, (id, label, flags, checked, enabled)) in items.iter().enumerate() {
                let f = *flags | if *checked { MF_CHECKED } else { 0 } | if *enabled { 0 } else { MF_GRAYED };
                if f & MF_SEPARATOR == MF_SEPARATOR {
                    let _ = AppendMenuW(hmenu, MENU_ITEM_FLAGS(f), 0, PCWSTR::null());
                } else {
                    labels.push(win::wide(label));
                    let n = labels.len() - 1;
                    let _ = AppendMenuW(
                        hmenu,
                        MENU_ITEM_FLAGS(f),
                        *id,
                        PCWSTR::from_raw(labels[n].as_ptr()),
                    );
                }
            }
            let mut pos = POINT::default();
            let _ = GetCursorPos(&mut pos);
            // Needed so the menu closes when you click somewhere else.
            let _ = SetForegroundWindow(self.hwnd);
            let chosen = TrackPopupMenu(
                hmenu,
                TRACK_POPUP_MENU_FLAGS(TPM_RIGHTBUTTON.0 | TPM_RETURNCMD.0 | TPM_NONOTIFY.0),
                pos.x,
                pos.y,
                None,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(hmenu);
            let event = match chosen.0 {
                n if n == MENU_ID_EDIT as i32 => Some(TrayEvent::EditRules),
                n if n == MENU_ID_LOG as i32 => Some(TrayEvent::OpenLog),
                n if n == MENU_ID_AUTOSTART as i32 => Some(TrayEvent::ToggleAutostart),
                n if n == MENU_ID_QUIT as i32 => Some(TrayEvent::Quit),
                _ => None,
            };
            if let Some(e) = event {
                let _ = self.tx_events.send(e);
            }
        }
    }

    fn drain(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                TrayCmd::SetTooltip(t) => self.tooltip = t,
                TrayCmd::SetNow(n) => self.now = n,
            }
        }
    }
}

unsafe extern "system" fn tray_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_CREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        // The box we handed to CreateWindowExW becomes the window's state.
        let ptr = cs.lpCreateParams as *mut TrayState;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        return LRESULT(0);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayState;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;
    match msg {
        WM_TRAY => {
            let event = lparam.0 as u32 & 0xFFFF;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                state.show_menu();
            } else if event == WM_LBUTTONUP {
                // Let the window it opens come to the front.
                let _ = SetForegroundWindow(state.hwnd);
                let _ = state.tx_events.send(TrayEvent::EditRules);
            }
            LRESULT(0)
        }
        WM_UPDATE_TIP => {
            state.notify(NIM_MODIFY);
            LRESULT(0)
        }
        WM_APP_CMDS => {
            state.drain();
            LRESULT(0)
        }
        WM_CLOSE => {
            state.notify(NIM_DELETE);
            let _ = DestroyWindow(state.hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(hicon) = state.hicon.take() {
                let _ = DestroyIcon(hicon);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        other => {
            if other == state.taskbar_created {
                // Explorer restarted: add the icon again.
                state.notify(NIM_ADD);
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
    }
}

fn run(
    rx: Receiver<TrayCmd>,
    tx_events: Sender<TrayEvent>,
    hwnd_holder: std::sync::Arc<std::sync::Mutex<Option<isize>>>,
    icon_path: Option<PathBuf>,
    ready_tx: Sender<Result<()>>,
) -> WinRes<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = win::wide("DiscordStatusTray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(tray_wndproc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: PCWSTR::from_raw(class.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
        // A regular (never shown) window rather than a message-only one, so it
        // gets the TaskbarCreated broadcast when Explorer restarts.
        let state = Box::new(TrayState {
            hwnd: HWND(std::ptr::null_mut()),
            hicon: None,
            taskbar_created: 0,
            tooltip: "Discord Status".into(),
            now: "starting\u{2026}".into(),
            rx,
            tx_events,
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(class.as_ptr()),
            win::pcw("Discord Status"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(HINSTANCE(hinst.0)),
            Some(Box::into_raw(state) as *const _),
        )?;
        let state = &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayState);
        state.hwnd = hwnd;
        state.taskbar_created = windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(
            win::pcw("TaskbarCreated"),
        );
        let cx = GetSystemMetrics(SM_CXSMICON);
        let cy = GetSystemMetrics(SM_CYSMICON);
        let mut hicon: Option<HICON> = None;
        if let Some(path) = icon_path.as_ref().filter(|p| p.exists()) {
            let w = win::wide(&path.to_string_lossy());
            if let Ok(h) = LoadImageW(
                None,
                PCWSTR::from_raw(w.as_ptr()),
                GDI_IMAGE_TYPE(IMAGE_ICON.0),
                cx,
                cy,
                IMAGE_FLAGS(LR_LOADFROMFILE.0),
            ) {
                hicon = Some(HICON(h.0));
            }
        }
        if hicon.is_none() {
            if let Ok(h) = LoadIconW(None, IDI_APPLICATION) {
                hicon = Some(h);
            }
        }
        state.hicon = hicon;
        if !state.notify(NIM_ADD) {
            let _ = ready_tx.send(Err(anyhow!("the shell wouldn't take the icon")));
            return Err(windows::core::Error::from_win32().into());
        }
        *hwnd_holder.lock().unwrap() = Some(hwnd.0 as isize);
        let _ = ready_tx.send(Ok(()));

        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg as *mut MSG, None, 0, 0);
            if !ret.as_bool() {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
