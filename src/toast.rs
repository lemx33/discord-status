//! The small popup in the corner when your status changes. A layered,
//! click-through Win32 window on its own thread, painted with plain GDI:
//! the same dim, soft-fading toast the Python original drew with Tk.

use crate::app;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use windows::{
    core::{PCWSTR, Result as WinRes},
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
            FillRect, FrameRect, SelectObject, SetBkMode, SetTextColor,
            DRAW_TEXT_FORMAT, FONT_CHARSET, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION,
            FONT_QUALITY, HBRUSH, HFONT, HGDIOBJ, TRANSPARENT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, PostMessageW,
            PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes, SetTimer,
            SetWindowLongPtrW, SetWindowPos, ShowWindow, GetWindowLongPtrW,
            CREATESTRUCTW, GWLP_USERDATA, HWND_TOP, LAYERED_WINDOW_ATTRIBUTES_FLAGS,
            MSG, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER,
            SWP_SHOWWINDOW, WM_APP, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
            WINDOW_EX_STYLE,
        },
        Graphics::Gdi::{GetDC, PAINTSTRUCT, ReleaseDC},
    },
};

const WM_APP_SHOW: u32 = WM_APP + 10;
const WM_APP_QUIT: u32 = WM_APP + 11;
const TIMER_FADE: usize = 1;

// Deliberately low-key: one dim line, translucent, short and soft fades.
const BG: COLORREF = COLORREF(0x001e1f22);
const BORDER: COLORREF = COLORREF(0x002b2d31);
const ACCENT: COLORREF = COLORREF(0x004e5058);
const FG: COLORREF = COLORREF(0x00b5bac1);
const MAX_ALPHA: u8 = 217; // 0.85
const FADE_IN_MS: u32 = 250;
const FADE_OUT_MS: u32 = 600;
const STEP_MS: u32 = 15;
const MAX_CHARS: usize = 70;

pub enum ToastCmd {
    Show { title: String, body: String, seconds: f64, position: String },
    Stop,
}

pub struct Toast {
    tx: Sender<ToastCmd>,
    hwnd: std::sync::Arc<std::sync::Mutex<Option<isize>>>,
    thread: Option<JoinHandle<()>>,
}

impl Toast {
    pub fn spawn() -> Toast {
        let (tx, rx) = mpsc::channel();
        let hwnd = std::sync::Arc::new(std::sync::Mutex::new(None));
        let hwnd2 = hwnd.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("toast".into())
            .spawn(move || {
                if let Err(e) = run(rx, hwnd2, ready_tx.clone()) {
                    app::log(&format!("toast disabled: {}", e));
                }
            })
            .ok();
        let _ = ready_rx.recv_timeout(std::time::Duration::from_secs(2));
        Toast { tx, hwnd, thread }
    }

    pub fn show(&self, title: &str, body: &str, seconds: f64, position: &str) {
        let _ = self.tx.send(ToastCmd::Show {
            title: title.to_string(),
            body: body.to_string(),
            seconds,
            position: position.to_string(),
        });
        self.wake();
    }

    pub fn stop(&mut self) {
        let _ = self.tx.send(ToastCmd::Stop);
        self.wake();
        if let Some(t) = self.thread.take() {
            // The window closes itself on the next message.
            let _ = t.join();
        }
    }

    fn wake(&self) {
        let hwnd = *self.hwnd.lock().unwrap();
        if let Some(h) = hwnd {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP_SHOW, WPARAM(0), LPARAM(0));
            }
        }
    }
}

struct ToastState {
    hwnd: HWND,
    dpi: u32,
    // Fade bookkeeping.
    alpha: u8,
    target: u8,
    step: u32,
    steps: u32,
    phase: u8, // 0 = idle, 1 = fading in, 2 = holding, 3 = fading out
    hold_left_ms: u32,
    // Layout.
    w: i32,
    h: i32,
    text: String,
    seconds: f64,
    position: String,
    font: windows::Win32::Graphics::Gdi::HFONT,
    visible: bool,
    rx: Receiver<ToastCmd>,
}

impl ToastState {
    fn scale(&self) -> f64 {
        self.dpi as f64 / 96.0
    }

    fn px(&self, n: f64) -> i32 {
        (n * self.scale()).round() as i32
    }
}

fn text_width_px(state: &ToastState, text: &str) -> (i32, i32) {
    unsafe {
        let hdc = GetDC(Some(state.hwnd));
        let font: HGDIOBJ = HGDIOBJ(state.font.0);
        let old = SelectObject(hdc, font);
        let mut wide = crate::win::wide(text);
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        DrawTextW(
            hdc,
            &mut wide,
            &mut r,
            DRAW_TEXT_FORMAT(0x00000400 | 0x0000), // DT_CALCRECT | DT_NOPREFIX
        );
        SelectObject(hdc, old);
        let _ = ReleaseDC(Some(state.hwnd), hdc);
        (r.right - r.left, r.bottom - r.top)
    }
}

fn layout(state: &mut ToastState) {
    let text = {
        let mut t = state.text.clone();
        let count = t.chars().count();
        if count > MAX_CHARS {
            t = t.chars().take(MAX_CHARS - 1).collect::<String>() + "\u{2026}";
        }
        t
    };
    let (tw, th) = text_width_px(state, &text);
    let pad_x = state.px(10.0);
    let pad_y = state.px(5.0);
    let bar = state.px(2.0);
    let margin = state.px(16.0);
    let w = tw + bar + pad_x * 2;
    let h = th + pad_y * 2;
    state.w = w;
    state.h = h;

    let area = crate::win::work_area();
    let x = if state.position.contains("left") {
        area.left + margin
    } else {
        area.right - w - margin
    };
    let y = if state.position.contains("top") {
        area.top + margin
    } else {
        area.bottom - h - margin
    };
    unsafe {
        // One pixel of breathing room, so the 1px border stays visible.
        let _ = SetWindowPos(
            state.hwnd,
            Some(HWND_TOP),
            x,
            y,
            w + 2,
            h + 2,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
    }
}

fn paint(state: &ToastState) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(state.hwnd, &mut ps);
        let r = RECT { left: 0, top: 0, right: state.w, bottom: state.h };
        FillRect(hdc, &r, state.bg_brush());
        FrameRect(hdc, &r, state.border_brush());
        // Accent bar down the left edge.
        let bar = RECT {
            left: 0,
            top: 0,
            right: state.px(2.0),
            bottom: state.h,
        };
        FillRect(hdc, &bar, state.accent_brush());
        // Text.
        let font: HGDIOBJ = HGDIOBJ(state.font.0);
        let old = SelectObject(hdc, font);
        let text = {
            let mut t = state.text.clone();
            if t.chars().count() > MAX_CHARS {
                t = t.chars().take(MAX_CHARS - 1).collect::<String>() + "\u{2026}";
            }
            t
        };
        let mut wide = crate::win::wide(&text);
        let pad_x = state.px(10.0);
        let bar = state.px(2.0);
        let mut tr = RECT {
            left: bar + pad_x,
            top: state.px(5.0),
            right: state.w - pad_x,
            bottom: state.h - state.px(5.0),
        };
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, FG);
        DrawTextW(
            hdc,
            &mut wide,
            &mut tr,
            DRAW_TEXT_FORMAT(0x0000 | 0x0020), // DT_LEFT | DT_TOP
        );
        SelectObject(hdc, old);
        let _ = EndPaint(state.hwnd, &ps);
    }
}

impl ToastState {
    unsafe fn bg_brush(&self) -> HBRUSH {
        CreateSolidBrush(BG)
    }
    unsafe fn border_brush(&self) -> HBRUSH {
        CreateSolidBrush(BORDER)
    }
    unsafe fn accent_brush(&self) -> HBRUSH {
        CreateSolidBrush(ACCENT)
    }
}

fn start_fade(state: &mut ToastState, target: u8, ms: u32) {
    state.target = target;
    state.steps = (ms / STEP_MS).max(1);
    state.step = 0;
    unsafe {
        let _ = SetTimer(Some(state.hwnd), TIMER_FADE, STEP_MS, None);
    }
}

fn on_show(state: &mut ToastState) {
    while let Ok(cmd) = state.rx.try_recv() {
        match cmd {
            ToastCmd::Show { title, body, seconds, position } => {
                state.text = if body.is_empty() {
                    title
                } else {
                    format!("{} \u{00b7} {}", title, body)
                };
                state.seconds = seconds;
                state.position = position;
            }
            ToastCmd::Stop => {
                unsafe {
                    let _ = PostMessageW(Some(state.hwnd), WM_APP_QUIT, WPARAM(0), LPARAM(0));
                }
                return;
            }
        }
    }
    if state.text.is_empty() {
        return;
    }
    unsafe {
        // A new change replaces the current popup.
        let _ = KillTimer(Some(state.hwnd), TIMER_FADE);
        layout(state);
        let _ = ShowWindow(state.hwnd, SW_SHOWNOACTIVATE);
        state.visible = true;
        let _ = SetLayeredWindowAttributes(
            state.hwnd,
            COLORREF(0),
            0,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS(0x2), // LWA_ALPHA
        );
        state.phase = 1;
        start_fade(state, MAX_ALPHA, FADE_IN_MS);
    }
}

fn on_timer(state: &mut ToastState) {
    state.step += 1;
    let t = (state.step as f64 / state.steps as f64).min(1.0);
    let alpha = if state.phase == 1 {
        state.alpha as f64 + (MAX_ALPHA as f64 - state.alpha as f64) * t
    } else {
        state.alpha as f64 - state.alpha as f64 * t
    };
    state.alpha = alpha as u8;
    unsafe {
        let _ = SetLayeredWindowAttributes(
            state.hwnd,
            COLORREF(0),
            state.alpha,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS(0x2),
        );
    }
    if state.step >= state.steps {
        unsafe {
            let _ = KillTimer(Some(state.hwnd), TIMER_FADE);
        }
        match state.phase {
            1 => {
                state.phase = 2;
                state.hold_left_ms = (state.seconds * 1000.0).max(0.0) as u32;
                unsafe {
                    // Hold, then fade out, all through the one timer.
                    let _ = SetTimer(Some(state.hwnd), TIMER_FADE, state.hold_left_ms.max(STEP_MS), None);
                }
            }
            2 => {
                state.phase = 3;
                start_fade(state, 0, FADE_OUT_MS);
            }
            _ => {
                state.phase = 0;
                if state.visible {
                    unsafe {
                        let _ = ShowWindow(state.hwnd, SW_HIDE);
                    }
                    state.visible = false;
                }
            }
        }
    }
}

unsafe extern "system" fn toast_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_CREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        let ptr = cs.lpCreateParams as *mut ToastState;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        return LRESULT(0);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ToastState;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;
    match msg {
        WM_PAINT => {
            paint(state);
            LRESULT(0)
        }
        WM_APP_SHOW => {
            on_show(state);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 as usize == TIMER_FADE {
                on_timer(state);
            }
            LRESULT(0)
        }
        WM_APP_QUIT | WM_CLOSE => {
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_FADE);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !state.font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(state.font.0));
            }
            let boxed = Box::from_raw(ptr);
            drop(boxed);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn run(rx: Receiver<ToastCmd>, hwnd_holder: std::sync::Arc<std::sync::Mutex<Option<isize>>>, ready: Sender<()>) -> WinRes<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = crate::win::wide("DiscordStatusToast");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(toast_wndproc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: PCWSTR::from_raw(class.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let state = Box::new(ToastState {
            hwnd: HWND(std::ptr::null_mut()),
            dpi: 96,
            alpha: 0,
            target: 0,
            step: 0,
            steps: 1,
            phase: 0,
            hold_left_ms: 0,
            w: 0,
            h: 0,
            text: String::new(),
            seconds: 2.0,
            position: "bottom-left".into(),
            font: HFONT(std::ptr::null_mut()),
            visible: false,
            rx,
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0),
            PCWSTR::from_raw(class.as_ptr()),
            crate::win::pcw("Discord Status"),
            WS_POPUP,
            0, 0, 1, 1,
            None, None, Some(HINSTANCE(hinst.0)),
            Some(Box::into_raw(state) as *const _),
        )?;
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ToastState;
        (*ptr).hwnd = hwnd;
        (*ptr).dpi = dpi;
        // Segoe UI at 9pt, sized for this monitor (the font copies the name).
        let mut face = crate::win::wide("Segoe UI");
        face.shrink_to_fit();
        let font = CreateFontW(
            -(9 * dpi as i32 / 72),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            FONT_CHARSET(1),
            FONT_OUTPUT_PRECISION(0),
            FONT_CLIP_PRECISION(0),
            FONT_QUALITY(4),
            0,
            PCWSTR::from_raw(Box::into_raw(face.into_boxed_slice()) as *const u16),
        );
        (*ptr).font = font;
        *hwnd_holder.lock().unwrap() = Some(hwnd.0 as isize);
        let _ = ready.send(());

        let mut msg = MSG::default();
        loop {
            let ret = windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg as *mut MSG, None, 0, 0);
            // 0 = quit, -1 = error
            if !ret.as_bool() {
                break;
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
    }
    Ok(())
}

