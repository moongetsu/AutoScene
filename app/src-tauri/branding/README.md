# Installer art

`render.py` draws the installer pictures from the mark (`app/public/autoscene-mark.svg`, geometry copied
unit for unit) and the colours in `docs/DESIGN.md`. The results are committed; a build doesn't run this.

From the repository root, after `npm ci` in `app/` (the title font comes from `@fontsource/instrument-serif`):

```bash
uv run --script app/src-tauri/branding/render.py
```

uv reads the exact Pillow version from the script's header and runs it in a throwaway environment.

| File | Where it shows | Size |
|---|---|---|
| `windows/sidebar.bmp` | NSIS welcome and finish pages | 328×628, 24-bit (2× of 164×314) |
| `windows/header.bmp` | NSIS page header, installer and uninstaller | 300×114, 24-bit (2× of 150×57) |
| `macos/dmg-background.png` | the DMG's Finder window | 1320×800 at 144 dpi = 660×400 pt |
| `icons/macos/icon.icns` | the Mac app and the DMG volume | 16–1024 px, 824 px tile in 1024 px |

The DMG art is laid out for the window size and icon places in `tauri.macos.conf.json` (128 pt icons, 16 pt
labels, the bundler's defaults): change those and the picture together. The plates under the two labels are a
mid grey (`#787570`) because Finder sets the labels black in light mode and white in dark mode; both read on it
at 4.6:1.

The Windows installer keeps the light Windows look on purpose: only pictures and icons, no MUI colours and no
own `nsis.template`. With visual styles the finish page's checkboxes ignore a text colour (black on dark), and
the pages in between would stay light anyway.
