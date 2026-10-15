# tauri-specta — AutoScene port to Tauri 3

Upstream: <https://github.com/specta-rs/tauri-specta> (MIT, see `LICENSE`), version 2.0.0-rc.25.
Upstream only supports Tauri 2; AutoScene runs on Tauri 3 (alpha). This copy is wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`.

Changes against the published crate:

- `Cargo.toml`: depends on `tauri =3.0.0-alpha.4` (with its `specta` feature, which still implements
  `FunctionArg` for `State`, `AppHandle`, `Window`, `Webview` and `WebviewWindow`); dev-dependencies,
  docs.rs metadata and lint tables dropped.
- Doc comments: `tauri::Wry` → `tauri::DynRuntime` (Tauri 3 removed `tauri::Wry`; the type-erased
  `DynRuntime` is the default runtime).

No source logic was changed. Remove this directory and the patch once upstream releases Tauri 3 support.
