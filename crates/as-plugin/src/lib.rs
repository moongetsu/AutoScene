//! Plugins: programs by other people that AutoScene runs on request, after
//! an export or from the command palette. A plugin describes itself in a
//! `plugin.json` ([`manifest`]); AutoScene draws what it adds (menu entries,
//! commands, settings) and runs it as a separate process. It comes as a
//! package ([`package`]): a zip whose `files.lock` lists and hashes each of
//! its files. Who signed that list decides the plugin's trust ([`trust`]):
//! AutoScene's review, its author's pinned key, or nobody AutoScene knows.
//!
//! What a plugin says it does (its permissions) is shown before install, and
//! is not enforced: a process plugin runs with the person's user rights.

pub mod error;
pub mod keys;
pub mod manifest;
pub mod package;
pub mod trust;

pub use error::{PluginError, PluginResult};
pub use manifest::{
    ActionSpec, Author, CommandSpec, Engines, Launch, Manifest, Parsed, Permission, PluginId, Runtime, SelectOption,
    SettingKind, SettingSpec, SettingValue, current_platform,
};
pub use package::{FilesLock, LockedFile, Unpacked};
pub use trust::{AuthorKey, Keys, Trust, Verdict};
