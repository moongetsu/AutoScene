//! Plugins: programs by other people that AutoScene runs on request, after
//! an export or from the command palette. A plugin describes itself in a
//! `plugin.json` ([`manifest`]); AutoScene draws what it adds (menu entries,
//! commands, settings) and runs it as a separate process.
//!
//! What a plugin says it does (its permissions) is shown before install, and
//! is not enforced: a process plugin runs with the person's user rights.

pub mod error;
pub mod manifest;
pub mod package;

pub use error::{PluginError, PluginResult};
pub use manifest::{
    ActionSpec, Author, CommandSpec, Engines, Launch, Manifest, Parsed, Permission, PluginId, Runtime, SelectOption,
    SettingKind, SettingSpec, SettingValue, current_platform,
};
