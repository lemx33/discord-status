//! Discord IPC over the local named pipe. Frames are int32 op, int32 length,
//! JSON body. This talks only to the Discord app on this PC: no account token,
//! no HTTP, nothing leaves the machine.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;

const OP_HANDSHAKE: i32 = 0;
const OP_FRAME: i32 = 1;
const OP_CLOSE: i32 = 2;

/// What went wrong: a bad activity payload (connection still fine) versus the
/// pipe dying (needs a reconnect).
#[derive(Debug)]
pub enum IpcError {
    BadActivity(String),
    Closed(String),
    Connection(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::BadActivity(m) => write!(f, "Discord rejected activity: {}", m),
            IpcError::Closed(m) => write!(f, "Discord closed the connection: {}", m),
            IpcError::Connection(m) => write!(f, "{}", m),
        }
    }
}

impl std::error::Error for IpcError {}

pub struct DiscordIpc {
    pipe: std::fs::File,
    client_id: String,
}

fn pipe_name(i: u32) -> String {
    format!("\\\\.\\pipe\\discord-ipc-{}", i)
}

impl DiscordIpc {
    pub fn connect(client_id: &str) -> Result<DiscordIpc> {
        let mut ipc = DiscordIpc {
            pipe: open_pipe()?,
            client_id: client_id.to_string(),
        };
        ipc.handshake()?;
        Ok(ipc)
    }

    fn handshake(&mut self) -> Result<()> {
        self.send(OP_HANDSHAKE, &json!({"v": 1, "client_id": self.client_id}))?;
        let (op, data) = self.recv()?;
        if op == OP_CLOSE || data.get("evt").and_then(Value::as_str) != Some("READY") {
            let e = IpcError::Connection(format!("handshake rejected: {}", data));
            return Err(anyhow!(e.to_string()));
        }
        Ok(())
    }

    pub fn set_activity(&mut self, activity: Option<&Value>) -> Result<(), IpcError> {
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": std::process::id(), "activity": activity},
            "nonce": uuid::Uuid::new_v4().to_string(),
        });
        self.send(OP_FRAME, &payload).map_err(|e| IpcError::Connection(e.to_string()))?;
        let (op, data) = self.recv().map_err(|e| IpcError::Connection(e.to_string()))?;
        if op == OP_CLOSE {
            return Err(IpcError::Closed(data.to_string()));
        }
        if data.get("evt").and_then(Value::as_str) == Some("ERROR") {
            return Err(IpcError::BadActivity(
                data.get("data").map(|d| d.to_string()).unwrap_or_default(),
            ));
        }
        Ok(())
    }

    fn send(&mut self, op: i32, payload: &Value) -> Result<()> {
        let body = serde_json::to_vec(payload)?;
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(body.len() as i32).to_le_bytes());
        frame.extend_from_slice(&body);
        self.pipe.write_all(&frame)?;
        self.pipe.flush()?;
        Ok(())
    }

    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; n];
        let mut got = 0;
        while got < n {
            match self.pipe.read(&mut out[got..]) {
                Ok(0) => return Err(anyhow!(IpcError::Connection("pipe closed".into()).to_string())),
                Ok(k) => got += k,
                Err(e) => return Err(anyhow!(e)),
            }
        }
        Ok(out)
    }

    fn recv(&mut self) -> Result<(i32, Value)> {
        let mut header = [0u8; 8];
        let mut got = 0;
        while got < 8 {
            match self.pipe.read(&mut header[got..]) {
                Ok(0) => return Err(anyhow!(IpcError::Connection("pipe closed".into()).to_string())),
                Ok(k) => got += k,
                Err(e) => return Err(anyhow!(e)),
            }
        }
        let op = i32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let length = i32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let body = self.read_exact(length)?;
        let data: Value = serde_json::from_slice(&body)?;
        Ok((op, data))
    }
}

/// Open the first free Discord IPC pipe, like the Python loop over 0..=9.
fn open_pipe() -> Result<std::fs::File> {
    let mut last_err = None;
    for i in 0..10 {
        let result = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE, like the original's r+b
            .open(pipe_name(i));
        match result {
            Ok(f) => return Ok(f),
            Err(e) => last_err = Some(e),
        }
    }
    let e = IpcError::Connection(format!(
        "Discord isn't running (no IPC pipe): {}",
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ));
    Err(anyhow!(e.to_string()))
}