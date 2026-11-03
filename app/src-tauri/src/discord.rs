use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscordActivityPayload {
    pub details: Option<String>,
    pub state: Option<String>,
    pub start_timestamp: Option<u64>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    pub buttons: Vec<DiscordButton>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscordButton {
    pub label: String,
    pub url: String,
}

enum RpcMessage {
    SetActivity(Option<DiscordActivityPayload>),
    Stop,
}

pub struct DiscordRpcManager {
    sender: Mutex<Option<Sender<RpcMessage>>>,
    active: Arc<AtomicBool>,
}

impl DiscordRpcManager {
    pub fn new() -> Self {
        Self { sender: Mutex::new(None), active: Arc::new(AtomicBool::new(false)) }
    }

    pub fn start(&self, client_id: Option<String>) {
        let mut guard = self.sender.lock();
        if guard.is_some() {
            return;
        }

        let cid = client_id.unwrap_or_else(|| "1567208524940058695".into());
        let (tx, rx) = mpsc::channel();
        *guard = Some(tx);
        self.active.store(true, Ordering::SeqCst);

        let active_flag = self.active.clone();
        thread::Builder::new()
            .name("discord-rpc".into())
            .spawn(move || {
                run_rpc_worker(cid, rx, active_flag);
            })
            .ok();
    }

    pub fn update(&self, payload: Option<DiscordActivityPayload>) {
        let guard = self.sender.lock();
        if let Some(tx) = &*guard {
            tx.send(RpcMessage::SetActivity(payload)).ok();
        }
    }

    pub fn stop(&self) {
        let mut guard = self.sender.lock();
        if let Some(tx) = guard.take() {
            tx.send(RpcMessage::Stop).ok();
        }
        self.active.store(false, Ordering::SeqCst);
    }
}

impl Default for DiscordRpcManager {
    fn default() -> Self {
        Self::new()
    }
}

fn run_rpc_worker(client_id: String, rx: Receiver<RpcMessage>, active: Arc<AtomicBool>) {
    let mut stream = connect_discord(&client_id);
    let mut current_activity: Option<DiscordActivityPayload>;

    while active.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(RpcMessage::Stop) => {
                if let Some(ref mut s) = stream {
                    send_clear(s);
                    send_close(s);
                }
                break;
            }
            Ok(RpcMessage::SetActivity(payload)) => {
                current_activity = payload;
                if stream.is_none() {
                    stream = connect_discord(&client_id);
                }
                if let Some(ref mut s) = stream {
                    if let Some(ref act) = current_activity {
                        if !send_activity(s, act) {
                            stream = None;
                        }
                    } else if !send_clear(s) {
                        stream = None;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Heartbeat / ping if needed
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if let Some(ref mut s) = stream {
        send_clear(s);
        send_close(s);
    }
}

#[cfg(unix)]
type IpcStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type IpcStream = std::fs::File;

fn get_ipc_candidates() -> Vec<std::path::PathBuf> {
    let mut list = Vec::new();
    #[cfg(windows)]
    {
        for i in 0..10 {
            list.push(std::path::PathBuf::from(format!(r"\\?\pipe\discord-ipc-{i}")));
        }
    }
    #[cfg(unix)]
    {
        for var in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
            if let Ok(val) = std::env::var(var) {
                let p = std::path::PathBuf::from(val);
                if p.is_dir() {
                    for i in 0..10 {
                        list.push(p.join(format!("discord-ipc-{i}")));
                    }
                }
            }
        }
        for i in 0..10 {
            list.push(std::path::PathBuf::from(format!("/tmp/discord-ipc-{i}")));
        }
    }
    list
}

fn connect_discord(client_id: &str) -> Option<IpcStream> {
    for path in get_ipc_candidates() {
        #[cfg(unix)]
        {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&path) {
                stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
                stream.set_write_timeout(Some(Duration::from_secs(3))).ok();
                let handshake = serde_json::json!({
                    "v": 1,
                    "client_id": client_id
                });
                if write_packet(&mut stream, OP_HANDSHAKE, &handshake) {
                    if let Some((op, _)) = read_packet(&mut stream) {
                        if op == OP_FRAME {
                            return Some(stream);
                        }
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            if let Ok(mut file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                let handshake = serde_json::json!({
                    "v": 1,
                    "client_id": client_id
                });
                if write_packet(&mut file, OP_HANDSHAKE, &handshake) {
                    if let Some((op, _)) = read_packet(&mut file) {
                        if op == OP_FRAME {
                            return Some(file);
                        }
                    }
                }
            }
        }
    }
    None
}

fn write_packet<W: Write>(w: &mut W, op: u32, payload: &serde_json::Value) -> bool {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    let len = bytes.len() as u32;
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&op.to_le_bytes());
    header[4..].copy_from_slice(&len.to_le_bytes());
    w.write_all(&header).is_ok() && w.write_all(&bytes).is_ok() && w.flush().is_ok()
}

fn read_packet<R: Read>(r: &mut R) -> Option<(u32, serde_json::Value)> {
    let mut header = [0u8; 8];
    r.read_exact(&mut header).ok()?;
    let op = u32::from_le_bytes(header[..4].try_into().ok()?);
    let len = u32::from_le_bytes(header[4..].try_into().ok()?) as usize;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    let val = serde_json::from_slice(&body).ok()?;
    Some((op, val))
}

fn send_activity<W: Write + Read>(w: &mut W, act: &DiscordActivityPayload) -> bool {
    let pid = std::process::id();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().to_string();

    let mut activity_obj = serde_json::Map::new();
    if let Some(ref d) = act.details {
        activity_obj.insert("details".into(), serde_json::Value::String(d.clone()));
    }
    if let Some(ref s) = act.state {
        activity_obj.insert("state".into(), serde_json::Value::String(s.clone()));
    }
    if let Some(ts) = act.start_timestamp {
        activity_obj.insert(
            "timestamps".into(),
            serde_json::json!({
                "start": ts
            }),
        );
    }

    let mut assets_obj = serde_json::Map::new();
    if let Some(ref img) = act.large_image {
        assets_obj.insert("large_image".into(), serde_json::Value::String(img.clone()));
    }
    if let Some(ref txt) = act.large_text {
        assets_obj.insert("large_text".into(), serde_json::Value::String(txt.clone()));
    }
    if let Some(ref img) = act.small_image {
        assets_obj.insert("small_image".into(), serde_json::Value::String(img.clone()));
    }
    if let Some(ref txt) = act.small_text {
        assets_obj.insert("small_text".into(), serde_json::Value::String(txt.clone()));
    }
    if !assets_obj.is_empty() {
        activity_obj.insert("assets".into(), serde_json::Value::Object(assets_obj));
    }

    if !act.buttons.is_empty() {
        let btns = act
            .buttons
            .iter()
            .take(2)
            .map(|b| {
                serde_json::json!({
                    "label": b.label,
                    "url": b.url
                })
            })
            .collect::<Vec<_>>();
        activity_obj.insert("buttons".into(), serde_json::Value::Array(btns));
    }

    let packet = serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": activity_obj
        },
        "nonce": nonce
    });

    write_packet(w, OP_FRAME, &packet)
}

fn send_clear<W: Write + Read>(w: &mut W) -> bool {
    let pid = std::process::id();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().to_string();
    let packet = serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid
        },
        "nonce": nonce
    });
    write_packet(w, OP_FRAME, &packet)
}

fn send_close<W: Write>(w: &mut W) {
    write_packet(w, OP_CLOSE, &serde_json::json!({}));
}
