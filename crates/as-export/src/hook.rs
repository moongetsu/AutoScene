//! Post-export hook runner.
//!
//! Executes external scripts or commands when an export completes, passing
//! a structured JSON payload and export metadata to scratch files / stdin,
//! with process group isolation and cancellation support.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use as_core::CancelToken;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{ExportError, ExportResult};

/// Metadata payload passed to a post-export hook command.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PostExportPayload {
    pub export_id: String,
    pub project_id: String,
    pub project_title: Option<String>,
    pub out_dir: String,
    pub clip_count: usize,
    pub output_files: Vec<String>,
    pub completed_at: u64,
}

impl PostExportPayload {
    /// Substitute placeholder tokens in a command string.
    ///
    /// Supported tokens:
    /// - `{out_dir}`: destination folder of the export
    /// - `{export_id}`: unique ID of the export
    /// - `{project_id}`: unique ID of the project
    /// - `{clip_count}`: number of clips exported
    /// - `{meta_path}`: path to the temporary JSON payload file
    pub fn substitute_tokens(&self, cmd: &str, meta_path: &Path) -> String {
        let mut replacements = HashMap::new();
        replacements.insert("{out_dir}", self.out_dir.as_str());
        replacements.insert("{export_id}", self.export_id.as_str());
        replacements.insert("{project_id}", self.project_id.as_str());
        let count_str = self.clip_count.to_string();
        replacements.insert("{clip_count}", count_str.as_str());
        let meta_str = meta_path.to_string_lossy();
        replacements.insert("{meta_path}", meta_str.as_ref());

        let mut result = cmd.to_string();
        for (token, val) in replacements {
            result = result.replace(token, val);
        }
        result
    }
}

/// Execute a post-export command hook with supervised execution and process group cleanup.
pub fn run_hook(
    command_str: &str,
    payload: &PostExportPayload,
    scratch_dir: &Path,
    cancel: &CancelToken,
    timeout: Option<Duration>,
) -> ExportResult<()> {
    let trimmed = command_str.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    // Ensure scratch directory exists
    fs::create_dir_all(scratch_dir)?;
    let meta_file = scratch_dir.join("export_meta.json");
    let json_bytes = serde_json::to_vec_pretty(payload)
        .map_err(|e| ExportError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    fs::write(&meta_file, json_bytes)?;

    let expanded_cmd = payload.substitute_tokens(trimmed, &meta_file);

    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &expanded_cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        // Check if the command starts with an existing file (like a script path with spaces)
        let trimmed_cmd = expanded_cmd.trim();
        let path = Path::new(trimmed_cmd);
        if path.exists() {
            c.arg("-c");
            c.arg("\"$1\"");
            c.arg("--");
            c.arg(trimmed_cmd);
        } else {
            c.args(["-c", trimmed_cmd]);
        }
        c
    };

    // Sanitize environment, providing minimal host context
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(temp) = std::env::var("TEMP") {
        cmd.env("TEMP", temp);
    } else if let Ok(tmp) = std::env::var("TMPDIR") {
        cmd.env("TMPDIR", tmp);
    }
    cmd.env("AUTOSCENE_META", &meta_file);
    cmd.current_dir(PathBuf::from(&payload.out_dir));

    let (status, _stdout, stderr) = as_media::proc::run_supervised(&mut cmd, cancel, timeout)?;

    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr);
        let code = status.code().map_or("terminated".into(), |c| c.to_string());
        return Err(ExportError::BadPreset(format!("Post-export hook exited with code {code}: {msg}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_template_tokens_accurately() {
        let payload = PostExportPayload {
            export_id: "exp-123".into(),
            project_id: "proj-456".into(),
            project_title: Some("Sample Movie".into()),
            out_dir: "/tmp/exports".into(),
            clip_count: 5,
            output_files: vec!["/tmp/exports/clip1.mp4".into()],
            completed_at: 1700000000,
        };

        let template = "echo 'Finished {clip_count} clips in {out_dir} (id: {export_id}, meta: {meta_path})'";
        let meta_path = Path::new("/tmp/exports/meta.json");
        let result = payload.substitute_tokens(template, meta_path);

        assert_eq!(
            result,
            "echo 'Finished 5 clips in /tmp/exports (id: exp-123, meta: /tmp/exports/meta.json)'"
        );
    }

    #[test]
    fn executes_simple_hook_successfully() {
        let temp = tempfile::tempdir().unwrap();
        let payload = PostExportPayload {
            export_id: "test-exp".into(),
            project_id: "test-proj".into(),
            project_title: None,
            out_dir: temp.path().to_string_lossy().into(),
            clip_count: 1,
            output_files: vec![],
            completed_at: 1000,
        };

        let cancel = CancelToken::new();
        let res = run_hook("echo hello", &payload, temp.path(), &cancel, Some(Duration::from_secs(5)));
        assert!(res.is_ok(), "hook execution failed: {:?}", res.err());
        assert!(temp.path().join("export_meta.json").exists());
    }

    #[test]
    fn hook_reports_failure_on_bad_exit() {
        let temp = tempfile::tempdir().unwrap();
        let payload = PostExportPayload {
            export_id: "test-exp".into(),
            project_id: "test-proj".into(),
            project_title: None,
            out_dir: temp.path().to_string_lossy().into(),
            clip_count: 1,
            output_files: vec![],
            completed_at: 1000,
        };

        let cancel = CancelToken::new();
        let res = run_hook("exit 1", &payload, temp.path(), &cancel, Some(Duration::from_secs(5)));
        assert!(res.is_err());
    }
}

