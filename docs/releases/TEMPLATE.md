<!--
The text of a release: copy this file to docs/releases/<tag>.md (for
example v1.0.2-BETA.md), replace X.Y.Z with the version and fill in the
rest. The release workflow publishes it with the tag and fails without it.
-->
AutoScene X.Y.Z … (a sentence or two: what this version brings).

## Downloads

| File | For |
| --- | --- |
| `AutoScene_X.Y.Z_x64-setup.exe` | Windows 10 and 11 (x64) |
| `AutoScene_X.Y.Z_aarch64.dmg` | macOS 13 or later on Apple Silicon |
| `AutoScene_X.Y.Z_amd64.AppImage` | Linux x64, any distribution |
| `AutoScene_X.Y.Z_amd64.deb` / `AutoScene-X.Y.Z-1.x86_64.rpm` | Debian and Ubuntu / Fedora and openSUSE (needs WebKitGTK 4.1) |
| `SHA256SUMS` | The SHA-256 of every other file |

## What's new

- **…** …

## Before you install

- **Already installed?** The app updates itself (**Settings → Updates**).
  Your projects and models are kept.
- **Not signed by a publisher yet,** so the first start needs your OK once.
  On Windows, SmartScreen asks first: choose **More info → Run anyway** (with
  Smart App Control on, Windows 11 blocks the installer without that choice).
  On macOS, drag AutoScene to Applications, open it and close the message
  that macOS can't verify it. Then open **System Settings → Privacy &
  Security**, scroll down to the note about AutoScene, choose **Open Anyway**
  and confirm. (Since macOS 15, right-click → Open no longer does this.) If
  macOS later refuses a part of the app, such as `libonnxruntime.dylib`, run
  `xattr -dr com.apple.quarantine /Applications/AutoScene.app` once.

## Check your download

Each file comes with a signed record (a build attestation) of the workflow
of [tcmmi/AutoScene](https://github.com/tcmmi/AutoScene) that built it, and
the tag and commit it was built from. To check one, use the
[GitHub CLI](https://cli.github.com), signed in with any GitHub account:

```bash
gh attestation verify AutoScene_X.Y.Z_x64-setup.exe -R tcmmi/AutoScene
```

To insist on this tag and the release workflow, add
`--source-ref refs/tags/vX.Y.Z --signer-workflow tcmmi/AutoScene/.github/workflows/release.yml`.

Or compare SHA-256 hashes: put `SHA256SUMS` next to your downloads and run
`sha256sum --check --ignore-missing SHA256SUMS` (Linux) or
`shasum -a 256 --check --ignore-missing SHA256SUMS` (macOS). In PowerShell
on Windows, this prints `True` for an intact download:

```powershell
(Get-FileHash .\AutoScene_X.Y.Z_x64-setup.exe).Hash -eq (-split (Select-String -Path SHA256SUMS -Pattern x64-setup.exe -SimpleMatch).Line)[0]
```
