//! The host–plugin protocol, version 1 ([`crate::manifest::PROTOCOL`]): JSON
//! lines in UTF-8, one object per line, tagged by `"type"`, with camelCase
//! fields.
//!
//! A run starts with one [`HostMessage::Run`] on the plugin's stdin: what to
//! do, with the context it needs (the export, the project, the plugin's
//! settings, where FFmpeg is). A [`HostMessage::Cancel`] may follow. The
//! plugin answers on stdout with [`PluginMessage`]s: progress and log lines,
//! then one result or error line, and exits. The first such line counts, and
//! its stdin closes once it is in. A line that isn't a message this
//! AutoScene knows (not an object, or an unknown `type`) goes to the
//! plugin's log and is never an error, so a plugin may print what it likes;
//! stderr goes to the log too.
//!
//! Fields added later are `#[serde(default)]`, so an older plugin's lines
//! still read.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::manifest::{PluginId, SettingValue};

/// Host → plugin, on its stdin.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HostMessage {
    /// What to do: the first line of every run, and the only one but
    /// `cancel`.
    Run(Box<RunMessage>),
    /// Stop soon. A plugin that is still running [`crate::runtime::CANCEL_GRACE`]
    /// later is ended, with everything it started.
    Cancel,
}

impl HostMessage {
    /// The message as its line, without the newline.
    pub fn line(&self) -> String {
        // Plain data: maps with string keys, and floats serde_json writes
        // as null when they aren't finite.
        serde_json::to_string(self).expect("host messages always serialise")
    }
}

/// A run's task and its context.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunMessage {
    /// The protocol version the app speaks.
    pub protocol: u32,
    /// Unique per run; it heads the run's part of the log.
    pub run_id: String,
    pub plugin: PluginContext,
    pub app: AppContext,
    pub tools: ToolPaths,
    /// The plugin's settings, each of them, set or default.
    pub settings: BTreeMap<String, SettingValue>,
    pub action: ActionContext,
    /// The project it runs in, if any (palette commands may run without).
    pub project: Option<ProjectContext>,
    /// The export an export action runs on.
    pub export: Option<ExportContext>,
}

/// The plugin as installed.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginContext {
    pub id: PluginId,
    pub version: String,
    /// A folder the plugin may keep files in; runs start there.
    pub data_dir: String,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppContext {
    pub version: String,
    /// See [`crate::manifest::current_platform`].
    pub platform: String,
}

/// The app's FFmpeg, which a plugin may run itself.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolPaths {
    pub ffmpeg: String,
    pub ffprobe: String,
    pub ffmpeg_version: String,
}

/// What the person asked for, and how.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionContext {
    pub kind: ActionKind,
    /// The contribution's id in the manifest.
    pub id: String,
    pub trigger: Trigger,
}

/// Which of a plugin's contributions a run is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    ExportAction,
    Command,
}

/// What started a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    /// The export's menu.
    Menu,
    /// An export that finished, with the action turned on to run after each.
    AfterExport,
    /// The command palette.
    Palette,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    pub id: String,
    pub title: String,
    pub folder: String,
}

/// An export and its clips.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportContext {
    pub id: String,
    pub label: Option<String>,
    pub preset: String,
    pub out_dir: String,
    /// Unix milliseconds.
    pub created_at: i64,
    /// The one file a merged export wrote, if it was merged.
    pub merged_file: Option<String>,
    pub clips: Vec<ClipContext>,
}

/// One clip of an export, and the part of its episode it shows.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipContext {
    pub name: String,
    /// Where the clip was written, once it was.
    pub path: Option<String>,
    pub status: String,
    pub media_id: String,
    /// The episode's file.
    pub source_path: String,
    pub episode: String,
    /// The first frame of the episode the clip shows.
    pub start: u64,
    /// The frame after its last one.
    pub end: u64,
    /// The episode's frame rate as a fraction (`24000/1001`), if known.
    pub fps: Option<String>,
}

/// Plugin → host, on its stdout.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginMessage {
    /// How far the run is (`fraction`, 0 to 1, or not known) and what it
    /// does now, shown in Activity. Without a stage the plugin's name shows.
    Progress {
        #[serde(default)]
        fraction: Option<f64>,
        #[serde(default)]
        stage: Option<String>,
    },
    /// A line for the plugin's log.
    Log {
        #[serde(default)]
        level: LogLevel,
        message: String,
    },
    /// The run did its work. The message, if any, is shown to the person
    /// ("Posted 12 clips.").
    Result {
        #[serde(default)]
        message: Option<String>,
    },
    /// The run failed. The message is shown to the person; the detail goes
    /// to the logs.
    Error {
        message: String,
        #[serde(default)]
        detail: Option<String>,
    },
}

impl PluginMessage {
    /// The message a line of the plugin's stdout holds, if it holds one.
    /// Only an object is: serde would also read `["result"]` as one. A
    /// byte order mark before it is skipped, since some programs (.NET
    /// ones told to write UTF-8) write one first.
    pub fn read(line: &str) -> Option<Self> {
        let line = line.strip_prefix('\u{FEFF}').unwrap_or(line);
        if !line.trim_start().starts_with('{') {
            return None;
        }
        serde_json::from_str(line).ok()
    }
}

/// How much a log line matters. A level this AutoScene doesn't know reads
/// as `info`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Warn,
    Error,
    // Last: serde takes the unknown ones only into the last variant.
    #[default]
    #[serde(other)]
    Info,
}

impl LogLevel {
    pub fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> RunMessage {
        RunMessage {
            protocol: 1,
            run_id: "run-1".into(),
            plugin: PluginContext {
                id: PluginId::parse("dev.autoscene.hello").unwrap(),
                version: "1.2.0".into(),
                data_dir: "/data".into(),
            },
            app: AppContext { version: "1.0.3-BETA".into(), platform: "linux-x86_64".into() },
            tools: ToolPaths { ffmpeg: "/ffmpeg".into(), ffprobe: "/ffprobe".into(), ffmpeg_version: "9.0".into() },
            settings: BTreeMap::new(),
            action: ActionContext { kind: ActionKind::Command, id: "hello".into(), trigger: Trigger::Palette },
            project: None,
            export: None,
        }
    }

    #[test]
    fn host_messages_are_one_line_each() {
        assert_eq!(HostMessage::Cancel.line(), r#"{"type":"cancel"}"#);
        let line = HostMessage::Run(Box::new(run())).line();
        assert!(line.starts_with(r#"{"type":"run","protocol":1,"runId":"run-1","plugin":{"id":"dev.autoscene.hello""#));
        assert!(line.ends_with(
            r#""action":{"kind":"command","id":"hello","trigger":"palette"},"project":null,"export":null}"#
        ));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn settings_travel_as_plain_json_values() {
        let mut message = run();
        message.settings = BTreeMap::from([
            ("count".into(), SettingValue::Number(3.0)),
            ("loud".into(), SettingValue::Bool(true)),
            ("name".into(), SettingValue::Text("Line\n2".into())),
            ("odd".into(), SettingValue::Number(f64::NAN)),
        ]);
        let line = HostMessage::Run(Box::new(message)).line();
        assert!(line.contains(r#""settings":{"count":3.0,"loud":true,"name":"Line\n2","odd":null}"#), "{line}");
    }

    #[test]
    fn actions_and_triggers_are_camel_case() {
        assert_eq!(serde_json::to_string(&ActionKind::ExportAction).unwrap(), r#""exportAction""#);
        assert_eq!(serde_json::to_string(&Trigger::AfterExport).unwrap(), r#""afterExport""#);
        assert_eq!(serde_json::from_str::<Trigger>(r#""menu""#).unwrap(), Trigger::Menu);
        assert_eq!(serde_json::from_str::<ActionKind>(r#""command""#).unwrap(), ActionKind::Command);
    }

    #[test]
    fn plugin_lines_read_with_their_defaults() {
        let read = |line: &str| PluginMessage::read(line).unwrap();
        assert_eq!(read(r#"{"type":"progress"}"#), PluginMessage::Progress { fraction: None, stage: None });
        assert_eq!(
            read(r#"{"type":"progress","fraction":0.25,"stage":"Posting","later":1}"#),
            PluginMessage::Progress { fraction: Some(0.25), stage: Some("Posting".into()) }
        );
        assert_eq!(
            read(r#"{"type":"log","message":"hi"}"#),
            PluginMessage::Log { level: LogLevel::Info, message: "hi".into() }
        );
        assert_eq!(
            read(r#"{"type":"log","level":"warn","message":"hi"}"#),
            PluginMessage::Log { level: LogLevel::Warn, message: "hi".into() }
        );
        assert_eq!(
            read(r#"{"type":"log","level":"loud","message":"hi"}"#),
            PluginMessage::Log { level: LogLevel::Info, message: "hi".into() }
        );
        assert_eq!(read(r#"{"type":"result"}"#), PluginMessage::Result { message: None });
        assert_eq!(
            read(r#"{"type":"result","message":"Posted 12 clips."}"#),
            PluginMessage::Result { message: Some("Posted 12 clips.".into()) }
        );
        assert_eq!(
            read(r#"{"type":"error","message":"No webhook."}"#),
            PluginMessage::Error { message: "No webhook.".into(), detail: None }
        );
        assert_eq!(read("\u{FEFF}{\"type\":\"result\"}"), PluginMessage::Result { message: None });
    }

    #[test]
    fn other_lines_are_not_messages() {
        for line in [
            "hello",
            "",
            r#"{"type":"future","x":1}"#,
            r#"{"message":"no type"}"#,
            r#"{"type":"error"}"#,
            r#"{"type":"log"}"#,
            r#"{"type":"progress","fraction":"half"}"#,
            r#"["result"]"#,
            r#"["error","Something broke."]"#,
            "null",
        ] {
            assert_eq!(PluginMessage::read(line), None, "{line}");
        }
        assert!(PluginMessage::read(r#"  {"type":"result"}"#).is_some());
    }
}
