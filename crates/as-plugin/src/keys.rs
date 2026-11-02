//! The keys AutoScene reviews plugins with, compiled into the app: a
//! package one of them signed is Reviewed (see [`crate::trust`]).

/// The AutoScene Plugins keys (current, then next), each the base64 line of
/// a minisign `.pub` file. Not the updater key. Empty until the first
/// review.
pub const REVIEW_KEYS: &[&str] = &[];
