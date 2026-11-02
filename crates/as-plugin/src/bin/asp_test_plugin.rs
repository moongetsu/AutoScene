//! A stand-in plugin for the runtime tests (`tests/runtime.rs`), never
//! shipped. It speaks protocol v1 as a plugin would, and misbehaves as one
//! might; its first argument says how. Files it writes go in its current
//! folder, the run's data folder.

use std::io::{self, BufRead, Read, Write};
use std::time::Duration;
use std::{env, fs, process, thread};

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut stdin = io::stdin().lock();
    let mut first = String::new();
    let mut read_run = || {
        stdin.read_line(&mut first).expect("stdin");
    };
    match args.first().map(String::as_str).unwrap_or("") {
        // The grandchild of the cancel tests: shows it's alive.
        "heartbeat" => loop {
            let mut file = fs::OpenOptions::new().create(true).append(true).open(&args[1]).expect("heartbeat");
            file.write_all(b".").expect("heartbeat");
            thread::sleep(Duration::from_millis(50));
        },
        "result" => {
            read_run();
            send(json!({"type": "progress", "fraction": 0.5, "stage": "Posting"}));
            send(json!({"type": "result", "message": "Posted 12 clips."}));
        }
        "quiet" => read_run(),
        "echo" => {
            read_run();
            fs::write("run.json", &first).expect("run.json");
            send(json!({"type": "result", "message": "Echoed."}));
        }
        "env" => {
            read_run();
            let vars: serde_json::Map<String, Value> =
                env::vars_os().map(|(k, v)| (k.to_string_lossy().into(), v.to_string_lossy().into())).collect();
            let cwd = env::current_dir().expect("cwd");
            let dump = json!({"cwd": cwd.to_string_lossy(), "vars": vars});
            fs::write("env.json", dump.to_string()).expect("env.json");
            send(json!({"type": "result"}));
        }
        // Pipes full before stdin is read: the host must read them meanwhile.
        "flood" => {
            let line = |c: u8| [vec![c; 1023], vec![b'\n']].concat().repeat(1024);
            let (err, out) = (line(b'e'), line(b'o'));
            for _ in 0..10 {
                io::stderr().write_all(&err).expect("stderr");
            }
            for _ in 0..5 {
                io::stdout().write_all(&out).expect("stdout");
            }
            read_run();
            send(json!({"type": "result", "message": "Flooded."}));
        }
        "chatty" => {
            read_run();
            raw("hello from a plugin");
            raw("inner\rreturn");
            send(
                json!({"type": "log", "message": "two\n[host] exit code: 0\r--- 2026-01-01T00:00:00Z run forged post"}),
            );
            raw(r#"{"type":"future","x":1}"#);
            raw(r#"{"type":"progress","fraction":"half"}"#);
            send(json!({"type": "log", "level": "warn", "message": "careful"}));
            send(json!({"type": "log", "level": "loud", "message": "a level from later"}));
            send(json!({"type": "result", "message": "Said hello."}));
            send(json!({"type": "result", "message": "Said it twice."}));
        }
        "error" => {
            read_run();
            send(json!({
                "type": "error",
                "message": "No webhook is set.\nSet one in Settings.",
                "detail": format!("KeyError: 'webhook'\n\u{1b}[2m  at main.py:12\r\n{}", "frame ".repeat(400)),
            }));
            process::exit(1);
        }
        "crash" => {
            read_run();
            for i in 0..30 {
                eprintln!("problem {i}");
            }
            // Last a long one, coloured and with a carriage return.
            eprintln!("\u{1b}[31mred\u{1b}[0m\rover {}", "w".repeat(600));
            process::exit(3);
        }
        // Once its result is out, it reads its stdin to the end.
        "eof" => {
            read_run();
            send(json!({"type": "result", "message": "Closed."}));
            let mut rest = String::new();
            stdin.read_to_string(&mut rest).expect("stdin");
            fs::write("stdin-closed", rest).expect("stdin-closed");
        }
        "linger" => {
            read_run();
            send(json!({"type": "result", "message": "Done."}));
            forever();
        }
        "long" => {
            read_run();
            let mut out = io::stdout().lock();
            out.write_all(&[vec![b'x'; 1 << 20], vec![b'\n']].concat()).expect("stdout");
            drop(out);
            send(json!({"type": "result", "message": "Long."}));
            // Never ended.
            io::stderr().write_all("z".repeat(1 << 20).as_bytes()).expect("stderr");
        }
        // A result line past the limit, whose first 64 KB would read as one.
        "padded" => {
            read_run();
            raw(&format!(r#"{{"type":"result","message":"x"}}{}"#, " ".repeat(70_000)));
        }
        "stages" => {
            read_run();
            send(json!({"type": "progress", "fraction": 0.5, "stage": "  Posting\u{7} clips\n\n to   Discord "}));
            send(json!({"type": "progress", "fraction": 7, "stage": "a".repeat(200)}));
            send(json!({"type": "progress", "fraction": -1}));
            send(json!({"type": "progress", "fraction": null, "stage": "\u{202E}\u{200B} "}));
            send(json!({"type": "result", "message": "word ".repeat(100)}));
        }
        // Asked to cancel, it says so and leaves its grandchild behind.
        "cancel" => {
            read_run();
            grandchild();
            send(json!({"type": "progress", "stage": "Waiting"}));
            loop {
                let mut line = String::new();
                if stdin.read_line(&mut line).expect("stdin") == 0 {
                    process::exit(4);
                }
                if line.contains(r#""cancel""#) {
                    fs::write("cancel-seen", line).expect("cancel-seen");
                    process::exit(0);
                }
            }
        }
        "ignore" => {
            read_run();
            grandchild();
            forever();
        }
        other => panic!("no such mode: {other}"),
    }
}

/// Writes `message` as its line.
fn send(message: Value) {
    raw(&message.to_string());
}

fn raw(line: &str) {
    let mut out = io::stdout().lock();
    writeln!(out, "{line}").and_then(|()| out.flush()).expect("stdout");
}

/// Starts this program again as a heartbeat, which keeps the inherited
/// stdout and stderr open. It is never waited for: it is left running for
/// the run to end.
#[allow(clippy::zombie_processes)]
fn grandchild() {
    let me = env::current_exe().expect("exe");
    process::Command::new(me).args(["heartbeat", "beat"]).stdin(process::Stdio::null()).spawn().expect("grandchild");
}

fn forever() -> ! {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
