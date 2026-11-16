//! Paths, logging, first-run helpers: config file location, Application ID,
//! "start with Windows", and showing files. Ported from status.py's helpers.

use crate::config;
use crate::win;
use anyhow::{Context, Result};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub const PLACEHOLDER_ID: &str = "PASTE_YOUR_APPLICATION_ID_HERE";

pub struct Paths {
    /// Where config.json and status.log live: %APPDATA%\DiscordStatus.
    pub here: PathBuf,
    pub config: PathBuf,
    pub example: PathBuf,
    pub icon: PathBuf,
    pub log: PathBuf,
}

pub fn paths() -> &'static Paths {
    static PATHS: OnceLock<Paths> = OnceLock::new();
    PATHS.get_or_init(|| {
        let here = config::app_dir();
        // The icon and the example config ship next to the executable (for a
        // source run, next to the current directory works too).
        let bundle = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let example = pick_file(&[
            bundle.join("config.example.json"),
            cwd.join("config.example.json"),
        ])
        .unwrap_or_else(|| bundle.join("config.example.json"));
        let icon = pick_file(&[bundle.join("icon.ico"), cwd.join("icon.ico")])
            .unwrap_or_else(|| bundle.join("icon.ico"));
        Paths {
            config: here.join("config.json"),
            log: here.join("status.log"),
            here,
            example,
            icon,
        }
    })
}

fn pick_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.into_iter().find(|p| p.exists()).cloned()
}

/// "2026-11-15 14:03:59" in local time.
pub fn local_datetime() -> String {
    let st = win::local_systemtime();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

pub fn log(msg: &str) {
    let line = format!("{} {}", local_datetime(), msg);
    if win::console_attached() {
        // Keep it on one line so the log stays greppable.
        println!("{}", line.replace('\n', " · "));
    }
    let p = paths();
    let _ = fs::create_dir_all(&p.here);
    if let Ok(meta) = fs::metadata(&p.log) {
        if meta.len() > 256_000 {
            let _ = fs::rename(&p.log, p.log.with_extension("log.old"));
        }
    }
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p.log)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.replace('\n', "\n           ").as_bytes())?;
            f.write_all(b"\n")
        });
}

/// Copy config.example.json on first run.
pub fn ensure_config() -> Result<()> {
    if !paths().config.exists() {
        fs::copy(&paths().example, &paths().config).with_context(|| {
            format!(
                "copying {} to {}",
                paths().example.display(),
                paths().config.display()
            )
        })?;
        log("created config.json from config.example.json");
    }
    Ok(())
}

/// The client id, or "" if it's missing or still the placeholder.
pub fn current_client_id(cfg: &config::Config) -> String {
    let value = cfg.get_str("client_id");
    if value == PLACEHOLDER_ID {
        String::new()
    } else {
        value
    }
}

/// Write client_id into config.json, keeping the rest of the file as-is.
pub fn save_client_id(client_id: &str) -> Result<()> {
    let path = paths().config.clone();
    let text = fs::read_to_string(&path).unwrap_or_default();
    let re = regex::Regex::new(r#"("client_id"\s*:\s*)"[^"]*""#).unwrap();
    if let Some(caps) = re.captures(&text) {
        // Swap in the new ID, leave every other byte of the file untouched.
        let replaced = re.replacen(
            &text,
            1,
            |caps: &regex::Captures| {
                format!("{}\"{}\"", &caps[1], client_id)
            },
        );
        let _ = caps;
        return write_text(&path, &replaced);
    }
    let mut data: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
    if let Some(map) = data.as_object_mut() {
        map.insert("client_id".into(), serde_json::json!(client_id));
    }
    let pretty = serde_json::to_string_pretty(&data)?;
    write_text(&path, &pretty)
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text)?;
    Ok(())
}

pub fn startup_shortcut() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths().here.clone());
    base.join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("Discord Status.lnk")
}

pub fn autostart_enabled() -> bool {
    startup_shortcut().exists()
}

/// Create or remove the Startup shortcut (PowerShell writes the .lnk the same
/// way the Python original did: through the WScript.Shell COM object).
pub fn set_autostart(enabled: bool) -> Result<()> {
    let path = startup_shortcut();
    if !enabled {
        if path.exists() {
            fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let target = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("DiscordStatus.exe"));
    let args = "--autostart".to_string();
    let dir = target
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:DS_LNK); \
                  $s.TargetPath = $env:DS_TARGET; \
                  $s.Arguments = $env:DS_ARGS; \
                  $s.WorkingDirectory = $env:DS_DIR; \
                  $s.Save()";
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("DS_LNK", &path)
        .env("DS_TARGET", &target)
        .env("DS_ARGS", &args)
        .env("DS_DIR", &dir)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .context("running PowerShell to create the Start Menu shortcut")?;
    if !status.success() {
        return Err(anyhow::anyhow!("PowerShell didn't create the shortcut"));
    }
    Ok(())
}

pub fn open_in_notepad(path: &Path) {
    let _ = Command::new("notepad.exe").arg(path).spawn();
}

pub fn open_url(url: &str) {
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .creation_flags(0x0800_0000)
        .spawn();
}