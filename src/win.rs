//! Thin Win32 wrappers: the foreground window and its program, idle time,
//! the visible window list, the single-instance mutex, the stop event and
//! small GDI helpers. Everything here was ctypes in the Python original.

use crate::rules::LocalTime;
use std::sync::OnceLock;
use windows::{
    core::{BOOL, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, RECT,
            SYSTEMTIME, TRUE, WAIT_EVENT,
        },
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
        System::{
            Console::GetConsoleWindow,
            SystemInformation::GetLocalTime,
            SystemInformation::GetTickCount,
            Threading::{
                CreateEventW, CreateMutexW, GetCurrentProcessId, OpenProcess,
                QueryFullProcessImageNameW, ResetEvent, SetEvent, WaitForSingleObject,
                PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            HiDpi::{SetProcessDpiAwareness, PROCESS_DPI_AWARENESS},
            Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{
                EnumWindows, GetForegroundWindow, GetWindowLongW, GetWindowTextLengthW,
                GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE,
                WS_EX_TOOLWINDOW,
            },
        },
    },
};

/// Build a NUL-terminated wide string.
pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// A NUL-terminated wide string that outlives the call. Used for window class
/// names and the like, created once per process: deliberately leaking the
/// buffer keeps the pointer valid (like the C APIs expect).
pub fn pcw(s: &str) -> PCWSTR {
    let mut v = wide(s);
    v.shrink_to_fit();
    PCWSTR::from_raw(Box::into_raw(v.into_boxed_slice()) as *const u16)
}

/// The foreground window: exe name, title, pid.
pub struct Foreground {
    pub exe: String,
    pub title: String,
    pub pid: u32,
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn window_title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n == 0 {
            return String::new();
        }
        let _ = n;
        from_wide(&buf)
    }
}

/// Full path -> base name, for a process we may or may not be allowed to open.
fn query_exe(pid: u32) -> String {
    unsafe {
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => return String::new(),
        };
        let mut buf = vec![0u16; 1024];
        let mut size: u32 = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        match ok {
            Ok(()) => {
                let path = from_wide(&buf[..size as usize]);
                path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()
            }
            Err(_) => String::new(),
        }
    }
}

pub fn foreground() -> Foreground {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Foreground { exe: String::new(), title: String::new(), pid: 0 };
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        Foreground {
            exe: query_exe(pid),
            title: window_title(hwnd),
            pid,
        }
    }
}

/// Seconds since the last keyboard or mouse input. GetTickCount wraps every
/// ~49 days; the masked subtraction keeps the difference sane.
pub fn idle_seconds() -> f64 {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if !GetLastInputInfo(&mut info).as_bool() {
            return 0.0;
        }
        let now = GetTickCount();
        ((now.wrapping_sub(info.dwTime)) & 0xFFFF_FFFF) as f64 / 1000.0
    }
}

// State for the EnumWindows callback (a plain fn pointer, so a thread-local
// stands in for a closure's captured state).
thread_local! {
    static ENUM_FOUND: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static ENUM_OWN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// (exe, title) for the visible app windows, for the rule editor's picker.
pub fn list_windows() -> Vec<(String, String)> {
    unsafe {
        // EnumWindows takes a plain fn pointer here, so the list lives in a
        // thread-local instead of a closure.
        ENUM_FOUND.with(|f| f.borrow_mut().clear());
        ENUM_OWN.with(|o| o.set(GetCurrentProcessId()));
        let _ = EnumWindows(Some(enum_windows_cb), LPARAM(0));
    }
    let list = ENUM_FOUND.with(|f| f.borrow().clone());
    let mut found: Vec<(String, String)> = Vec::new();
    for (exe, title) in list {
        if !found.contains(&(exe.clone(), title.clone())) {
            found.push((exe, title));
        }
    }
    found
}

unsafe extern "system" fn enum_windows_cb(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return TRUE;
        }
        let mut cloaked: u32 = 0; // hidden UWP windows still report visible
        let r = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = r;
        if cloaked != 0 {
            return TRUE;
        }
        let title = window_title(hwnd);
        if title.is_empty() {
            return TRUE;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let own = ENUM_OWN.with(|o| o.get());
        if pid == own {
            return TRUE;
        }
        let exe = query_exe(pid);
        if !exe.is_empty() {
            ENUM_FOUND.with(|f| f.borrow_mut().push((exe, title)));
        }
    }
    TRUE
}

/// Crisp windows on scaled displays. Ignore failures on old systems.
pub fn set_dpi_awareness() {
    unsafe {
        // 2 = PROCESS_PER_MONITOR_DPI_AWARE
        let _ = SetProcessDpiAwareness(PROCESS_DPI_AWARENESS(2));
    }
}

pub fn console_attached() -> bool {
    unsafe { !GetConsoleWindow().0.is_null() }
}

pub fn local_time() -> LocalTime {
    let st = local_systemtime();
    LocalTime {
        weekday: (st.wDayOfWeek as u32 + 6) % 7, // 0 = Sunday in SYSTEMTIME
        hour: st.wHour as u32,
        minute: st.wMinute as u32,
    }
}

pub fn local_systemtime() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

// ---------------------------------------------------------------------------
// Single instance + stop event
// ---------------------------------------------------------------------------

pub struct InstanceLock(HANDLE);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Claim the single-instance mutex, or None if another copy is already running
/// (letting go of it right away, so an open control panel can't block the next
/// start after the real one stops).
pub fn claim_instance() -> Option<InstanceLock> {
    unsafe {
        let name = wide("Local\\DiscordStatusPresence");
        let handle = CreateMutexW(None, false, PCWSTR::from_raw(name.as_ptr())).ok()?;
        let err = GetLastError();
        if err == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(InstanceLock(handle))
    }
}

/// A raw handle wrapped so it can live in a static (it is only ever used
/// from the one thread that owns the event).
#[derive(Clone, Copy)]
struct SyncHandle(isize);
unsafe impl Send for SyncHandle {}
unsafe impl Sync for SyncHandle {}

static STOP_EVENT: OnceLock<SyncHandle> = OnceLock::new();

/// A named event lets a second launch (the control window) stop the running copy.
pub fn stop_event() -> HANDLE {
    let sync = *STOP_EVENT.get_or_init(|| unsafe {
        let name = wide("Local\\DiscordStatusPresenceStop");
        let h = CreateEventW(None, true, false, PCWSTR::from_raw(name.as_ptr()))
            .unwrap_or(HANDLE(std::ptr::null_mut()));
        SyncHandle(h.0 as isize)
    });
    HANDLE(sync.0 as *mut _)
}

pub fn reset_stop_event() {
    unsafe {
        let _ = ResetEvent(stop_event());
    }
}

pub fn request_stop() {
    unsafe {
        let _ = SetEvent(stop_event());
    }
}

/// Sleep up to `ms`; true if a stop was requested.
pub fn wait_for_stop(ms: u32) -> bool {
    unsafe { WaitForSingleObject(stop_event(), ms) == WAIT_EVENT(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_event_signals() {
        reset_stop_event();
        assert!(!wait_for_stop(0), "not signalled after reset");
        request_stop();
        assert!(wait_for_stop(0), "signalled after request_stop");
        reset_stop_event();
        assert!(!wait_for_stop(0), "not signalled after second reset");
    }
}

/// The bounding rectangle of the work area (screen minus taskbar).
pub fn work_area() -> RECT {
    unsafe {
        let mut area = RECT::default();
        let ok = windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
            0,
            Some(&mut area as *mut RECT as *mut _),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        if ok.is_ok() {
            area
        } else {
            let cx = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
            );
            let cy = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN,
            );
            RECT { left: 0, top: 0, right: cx, bottom: cy }
        }
    }
}