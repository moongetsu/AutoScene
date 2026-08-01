# AutoScene updates

In-app updates for macOS and Windows, built on `tauri-plugin-updater`.

## What a member sees

AutoScene checks for a new version ~2.5s after launch, then every 6 hours while
it stays open. When one is found, a dialog shows the new version number, the
release date, and the release notes, with three choices:

| Choice | What it does |
| --- | --- |
| **Update Now** | Downloads and installs, then offers **Restart Now** to finish. |
| **Update Later** | Stops offering *this* version. Still installable from Check for Updates, and a **newer** version will prompt again. |
| **Remind Me Later** | Remembers nothing — asks again in 24 hours. |

Nothing downloads without that click. A failed automatic check is silent; a
failed manual check shows the reason.

**Check for Updates** is available two ways:

- macOS — the **AutoScene** menu, directly under About.
- Everywhere (incl. Windows) — **Settings → System → Updates**, which also
  shows the installed version, the last check time, and a switch to turn the
  automatic check off.

## Testing without a release server

Create `~/.autoscene/updates.json` (`%USERPROFILE%\.autoscene\updates.json` on
Windows):

```json
{
  "simulate": {
    "version": "0.2.0",
    "date": "2026-08-01",
    "notes": "• Faster CoreML scan startup\n• Fixes timeline scrubbing on 4K sources"
  }
}
```

Launch AutoScene. The prompt appears with that content, all three buttons work,
and **Update Now** runs the real progress bar against a simulated download —
then says so plainly instead of pretending to restart. Nothing on disk is
touched. Delete the file to go back to the live feed.

The same file can repoint the check at a staging server without simulating:

```json
{ "endpoint": "http://127.0.0.1:8000/latest.json" }
```

A missing or malformed file is ignored, so this never breaks a normal install.
The deferral state ("Update Later" / "Remind Me Later") lives in the webview's
`localStorage` under `autoscene.update.*`.

## Cutting a real release

1. Bump `version` in `src-tauri/tauri.conf.json`.

2. Build with the signing key present — `bundle.createUpdaterArtifacts` is on,
   so an unsigned build fails rather than shipping an unverifiable update:

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/autoscene-updater.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   npm run tauri:build
   ```

   It must be the key *contents*. `TAURI_SIGNING_PRIVATE_KEY_PATH` is not read
   by this CLI version — the build gets as far as the bundles and then fails
   with "a public key has been found, but no private key".

   macOS emits `AutoScene.app.tar.gz` + `.sig`, Windows `AutoScene_x64-setup.nsis.zip` + `.sig`,
   under `src-tauri/target/release/bundle/`.

3. Publish those archives, plus a `latest.json`, to the endpoint in
   `plugins.updater.endpoints` (currently the `tcmmi/AutoScene` GitHub
   releases `latest/download/` path):

   ```json
   {
     "version": "0.2.0",
     "notes": "• Faster CoreML scan startup\n• Fixes timeline scrubbing on 4K sources",
     "pub_date": "2026-08-01T10:00:00Z",
     "platforms": {
       "darwin-aarch64": {
         "signature": "<contents of AutoScene.app.tar.gz.sig>",
         "url": "https://github.com/tcmmi/AutoScene/releases/download/v0.2.0/AutoScene.app.tar.gz"
       },
       "darwin-x86_64": {
         "signature": "<contents of the x86_64 .sig>",
         "url": "https://github.com/tcmmi/AutoScene/releases/download/v0.2.0/AutoScene_x64.app.tar.gz"
       },
       "windows-x86_64": {
         "signature": "<contents of AutoScene_x64-setup.nsis.zip.sig>",
         "url": "https://github.com/tcmmi/AutoScene/releases/download/v0.2.0/AutoScene_0.2.0_x64-setup.nsis.zip"
       }
     }
   }
   ```

   `notes` is what the dialog shows verbatim — newlines are preserved.

### Keys

The keypair lives outside the repo at `~/.tauri/autoscene-updater.key{,.pub}`,
generated with `npx tauri signer generate`. The public half is already in
`tauri.conf.json`. **Keep the private key.** Losing it means no existing
install can verify a future update — recovery is a manual reinstall for
everyone.

## Platform notes

- **macOS** — the app replaces itself in place and relaunches. Ad-hoc-signed
  builds work; notarised builds are needed to avoid Gatekeeper warnings on
  first launch of a fresh download.
- **Windows** — `installMode: "passive"` shows the NSIS progress UI with no
  prompts. The installer exits the running app itself, so AutoScene does not
  call restart there.
- **Linux** — no updater artifacts are produced; the check reports
  up to date and the UI simply never prompts.

## Where the code is

| Path | Role |
| --- | --- |
| `src-tauri/src/updater.rs` | Check / install / relaunch commands, override file |
| `src-tauri/src/mac_menu.rs` | `Check for Updates…` menu item |
| `src/lib/updates.ts` | Invoke wrappers, deferral policy, version compare |
| `src/state/updates.tsx` | Shared check state, auto-check scheduling |
| `src/components/app/UpdateDialog.tsx` | The prompt and its five states |
| `src/components/settings/UpdatesPanel.tsx` | Settings → System → Updates |
