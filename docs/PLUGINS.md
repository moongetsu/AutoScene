# Writing AutoScene plugins

A plugin is a program that AutoScene runs for the person: on an export, from
the command palette, or by itself after every export. It describes itself in
a `plugin.json`, and AutoScene draws what it adds (menu entries, palette
commands, a settings page). It ships as a package, an `.asp` file, which the
`asp` tool makes and signs.

This guide is for plugin authors. It describes AutoScene's plugins as they
are now: version 1 of the manifest and of the protocol.

The design started from moongetsu's prototype in
[PR #93](https://github.com/tcmmi/AutoScene/pull/93): the review before an
install, the contributions AutoScene draws, a folder per plugin, a run whose
processes end together, and plugins that run after an export all come from
it, and so do the EDL and CSV exports.

## What a plugin can do

A version 1 plugin is a **process plugin**: AutoScene starts its program
once per run and talks to it over stdin and stdout. The program is either:

- a program in the plugin's own folder (an `.exe` on Windows), or
- a Python script, which [uv](https://docs.astral.sh/uv/) runs with the
  packages its lock pins.

It can add:

- **Export actions**: entries in each export's menu. The person can also
  turn each one on to run after every export.
- **Commands**: entries in the command palette, with or without a project.
- **Settings**: a page in Settings, which AutoScene draws and saves. Each
  run gets the values.

Each run is a job in Activity, with its progress, a Stop button, and the
plugin's message when it is done.

**A plugin runs with the person's user rights.** It can read, change and
send anything they can. The permissions a plugin names are shown before it
is installed, but nothing enforces them. Write a plugin so that what it does
matches what it says.

## Quick start

1. Install [uv](https://docs.astral.sh/uv/) and `asp` (from a checkout of
   this repository):

   ```bash
   cargo install --path crates/as-plugin --features cli --bin asp
   ```

2. Copy `examples/plugins/hello-export` to a folder of your own. In its
   `plugin.json`, change `id` to one of yours (`dev.yourname.my-plugin`),
   and `name`, `author` and `description` with it.
3. Lock the script: `uv lock --script main.py`. Do it again each time you
   change the `# /// script` block at the top of `main.py`.
4. In AutoScene, turn on **Settings → Plugins → Allow plugins that
   AutoScene has not reviewed**. Then **Load a folder…** and pick your
   folder. AutoScene runs it from there, as its files are, so a change to
   `main.py` counts at the next run.
5. Run it: **Write a clip list** is in each export's menu, and **Say hello**
   is in the palette (Ctrl+K, or ⌘K on a Mac). The result shows as a toast
   and in Activity. **Show its log** (right-click the plugin in Settings →
   Plugins) shows what else it wrote.
6. Make a package to share: `asp pack my-plugin my-plugin-1.0.0.asp`. Sign
   it with your key so its updates can be told from someone else's (see
   [Signing](#signing-with-asp)).

`examples/plugins/discord-notify` is a second example: a plugin with a
dependency that posts to the internet.

## How a run goes

1. The person starts an action, or an export finishes and an action runs
   after it. AutoScene queues a job, titled with the action's title, with
   the plugin's name under it.
2. When the job starts, AutoScene checks again that the plugin may run. It
   starts the program, in the plugin's data folder, and sends it the run
   line on stdin.
3. The plugin works, and writes progress and log lines to stdout. Activity
   shows "Starting <name>", then each stage the plugin names.
4. The plugin writes one result line, or one error line, and exits.
5. AutoScene shows the result's message in a toast and in Activity, or the
   error's message in a toast and as a failed job.

### Where it runs

- **The program** is started by its full path in the plugin's folder, with
  the arguments `plugin.json` gives. No shell reads the command. A uv
  script runs as `uv run --script --locked <the script's full path>`.
  AutoScene looks for uv on the PATH (but not in the Microsoft Store's
  `WindowsApps` folder), then in `~/.local/bin` and `~/.cargo/bin`; then in
  `%LOCALAPPDATA%\Microsoft\WinGet\Links` on Windows, and in
  `/opt/homebrew/bin` and `/usr/local/bin` on macOS and Linux. Apps started
  from the macOS Dock or Finder don't get the shell's PATH, which is why the
  folders count. Without uv, the run fails with "This plugin needs uv,
  which isn't installed. Install it from docs.astral.sh/uv, then try
  again."
- **The current folder** is the plugin's data folder (`plugin-data/<id>`,
  see [where AutoScene keeps plugins](#where-autoscene-keeps-plugins)). It
  is the plugin's to keep files in between runs. It stays through updates,
  and an uninstall removes it only when the person asks.
- **The environment** holds only what programs need to work:
  - On every system: `PATH`, `LANG`, `TZ`, the proxy variables
    (`http_proxy`, `https_proxy`, `no_proxy`, `all_proxy`, also in
    capitals), `CUDA_VISIBLE_DEVICES` and those that start with
    `CUDA_PATH`.
  - On Windows: `SystemRoot`, `windir`, `SystemDrive`, `PATHEXT`,
    `COMSPEC`, `TEMP`, `TMP`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`,
    `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`, `ProgramW6432`,
    `CommonProgramFiles`, `NUMBER_OF_PROCESSORS`, `PROCESSOR_ARCHITECTURE`,
    `PROCESSOR_ARCHITEW6432`, `OS`, `USERNAME`, `HOMEDRIVE` and `HOMEPATH`.
  - On macOS and Linux: `HOME`, `USER`, `LOGNAME`, `TMPDIR`, `DISPLAY`,
    `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, and those that start
    with `LC_` or `XDG_`.

  Every other variable is left out, also uv's own (`UV_CACHE_DIR`,
  `UV_INDEX_URL`); uv still reads its config files. AutoScene adds:

  | Variable | Value |
  |---|---|
  | `AUTOSCENE_PLUGIN_ID` | The plugin's id |
  | `AUTOSCENE_PLUGIN_DATA` | The data folder (also the current folder) |
  | `AUTOSCENE_FFMPEG`, `AUTOSCENE_FFPROBE` | AutoScene's FFmpeg and ffprobe |
  | `AUTOSCENE_PROTOCOL` | `1` |
  | `PYTHONUNBUFFERED`, `PYTHONUTF8` | `1` |
  | `PYTHONIOENCODING` | `utf-8` |

- **On Windows** it starts without a console window.

A plugin may run AutoScene's FFmpeg itself. The run line names it too.

### One at a time

Plugin runs wait for each other: one runs at a time, whatever plugin it
belongs to. A plugin with `"gpu": true` waits for the analysis of episodes
instead, and an analysis waits for it, so the two never share the graphics
card. Two runs of one plugin never run at once (Activity says "Waiting for
its other run"), and a run that starts while its plugin is updated waits
for the update, then runs the new version.

The engine keeps its models on the graphics card for up to 2 minutes after
its last analysis. A GPU plugin that starts right after an analysis may
find less free video memory than it expects.

A run that waits or runs already isn't queued again: the same action of the
same plugin, on the same export or in the same project. AutoScene then
says so.

There is no time limit. A plugin may run as long as it needs, and the
person can stop it in Activity.

### Cancel and the end of a run

- **Stop** in Activity sends the cancel line. A plugin that still runs 5
  seconds later is ended, with the processes it started. On Windows they
  all end at once. On macOS and Linux the plugin's process group gets
  SIGTERM, then SIGKILL 2 seconds later; a process that left the group
  (`setsid`, `setpgid`) keeps running. The run is cancelled, unless the
  plugin gave its result or error line first: Activity then shows that.
- After its result or error line, the plugin's stdin closes. A plugin that
  still runs 10 seconds later is ended the same way, and its line counts.
- AutoScene never tries a run again: what the plugin did, a post to a
  webhook say, would happen twice.
- Turning a plugin off stops its runs, running or still waiting. So does
  turning off "Allow plugins that AutoScene has not reviewed", for each
  plugin AutoScene hasn't reviewed. A plugin can't be uninstalled while it
  runs; its runs still waiting go with it.
- Quitting AutoScene stops the runs as Stop does. A run still waiting then
  is lost.

### After every export

Each export action can be turned on in the plugin's page in Settings,
under **After every export**. It is off until the person turns it on. It
then runs once an export finishes with every clip made: never after an
export that failed, was cancelled, or made only some of its clips. It runs
as a job of its own, so a plugin never fails an export or holds it up.

## The protocol (version 1)

The plugin and AutoScene talk in JSON lines: UTF-8, one JSON object per
line, each with a `"type"`. Fields are camelCase. The examples below are
spread over several lines to be read; each is sent as one line.

### The run line

The first line on the plugin's stdin is the run:

```json
{
  "type": "run",
  "protocol": 1,
  "runId": "01a19efa-56d7-7715-aa4f-e8fd5d5320d2",
  "plugin": {
    "id": "dev.autoscene.hello-export",
    "version": "1.0.0",
    "dataDir": "C:\\Users\\you\\AppData\\Local\\AutoScene\\plugin-data\\dev.autoscene.hello-export"
  },
  "app": { "version": "1.0.3-BETA", "platform": "windows-x86_64" },
  "tools": {
    "ffmpeg": "C:\\…\\ffmpeg\\ffmpeg.exe",
    "ffprobe": "C:\\…\\ffmpeg\\ffprobe.exe",
    "ffmpegVersion": "9.0"
  },
  "settings": { "format": "text", "greeting": "Hello", "withFrames": true },
  "action": { "kind": "exportAction", "id": "clip-list", "trigger": "menu" },
  "project": {
    "id": "019a2b3c-4d5e-7f60-8a9b-0c1d2e3f4a5b",
    "title": "Breaking Bad",
    "folder": "D:\\AutoScene Library\\Breaking Bad"
  },
  "export": {
    "id": "019a2b3c-9f8e-7d6c-b5a4-938271605f4e",
    "label": "Walter · scenepack",
    "preset": "prores-hq",
    "outDir": "D:\\Exports\\Walter",
    "createdAt": 1762150000000,
    "mergedFile": null,
    "clips": [
      {
        "name": "Walter_001",
        "path": "D:\\Exports\\Walter\\Walter_001.mov",
        "status": "done",
        "mediaId": "019a2b3d-0a1b-7c2d-8e3f-4a5b6c7d8e9f",
        "sourcePath": "D:\\Breaking Bad\\S01E01.mkv",
        "episode": "S01E01",
        "start": 1200,
        "end": 1450,
        "fps": "24000/1001"
      }
    ]
  }
}
```

| Field | What it holds |
|---|---|
| `protocol` | The protocol AutoScene speaks: `1`. |
| `runId` | A new id for each run. It also heads the run's part of the log. |
| `plugin` | The plugin's `id` and installed `version`, and `dataDir`: its data folder, where the run starts. |
| `app` | AutoScene's `version`, and the `platform` it runs on: `windows-x86_64`, `windows-aarch64`, `macos-aarch64`, `macos-x86_64`, `linux-x86_64` or `linux-aarch64`. |
| `tools` | AutoScene's `ffmpeg` and `ffprobe` (full paths) and FFmpeg's version. |
| `settings` | Each of the plugin's settings: the person's value, else its default. A setting with neither is left out. Numbers are JSON numbers and may come as `3.0`, also for whole-number settings. |
| `action` | What to do: `kind` is `exportAction` or `command`, `id` is the contribution's id in `plugin.json`, and `trigger` says what started it: `menu` (an export's menu), `afterExport` or `palette`. |
| `project` | The project it runs in: its `id`, `title` and `folder`. `null` for a command run without a project open. |
| `export` | The export an export action runs on; `null` for a command. |

An export holds its `id`, its `label` (`"Walter · scenepack"`, or `null`
for single clips), its `preset` (such as `prores-hq`, `h264-edit` or
`bit-perfect`), `outDir` (the folder it was written to), `createdAt` (Unix
milliseconds), `mergedFile` (the one file its clips were joined into, or
`null` for a file per clip) and its `clips`. Each clip has:

| Field | What it holds |
|---|---|
| `name` | The clip's file name, without its extension. |
| `path` | Where the clip was written; `null` until it is. In a joined export, the joined file. |
| `status` | `queued`, `running`, `done`, `failed` or `cancelled`. After every export, each is `done`. |
| `mediaId` | The episode's id in the project. |
| `sourcePath` | The episode's file. Empty when the episode was removed from the project since. |
| `episode` | The episode's name. Empty when it was removed. |
| `start`, `end` | The clip's first frame in the episode, and the frame after its last. |
| `fps` | The episode's frame rate as a fraction (`24000/1001`), or `null`. |

A scenepack export is joined by default: one file, with a chapter per clip.
(The run line above is from one made with a file per clip.) Then
`mergedFile` names that file, and each clip in it has that file as its
`path`. A clip's `start` and `end` are still frames of its episode, not
times in the joined file. Check `mergedFile` before you act on each clip's `path`,
or a plugin that uploads each clip uploads the whole file once per clip.

A later AutoScene may add fields: ignore the ones you don't know.

### The cancel line

The person pressed Stop:

```json
{"type":"cancel"}
```

Stop soon. Exit as you like: the run is cancelled whatever the exit.
AutoScene sends nothing else. Read stdin on a thread of its own (as the
examples do), or a cancel line waits until the plugin reads it.

### What the plugin writes

```json
{"type":"progress","fraction":0.25,"stage":"Posting clip 3 of 12"}
{"type":"log","level":"warn","message":"Clip 4 has no file; skipped."}
{"type":"result","message":"Posted 12 clips."}
{"type":"error","message":"Discord doesn't know that webhook.","detail":"HTTP Error 404: Not Found"}
```

- **`progress`**: `fraction` is how far the run is, from 0 to 1 (left out
  while it isn't known); `stage` is what it does now, shown in Activity.
  Both are optional; without a stage Activity shows the plugin's name. A
  fraction outside 0 to 1 is taken as the nearest end. A stage is cut to 80
  characters.
- **`log`**: a line for the plugin's log. `level` is `debug`, `info` (the
  default), `warn` or `error`; another reads as `info`.
- **`result`**: the run did its work. The optional `message` is shown to the
  person, in a toast and in Activity. Keep it to one short sentence; it is
  cut to 300 characters.
- **`error`**: the run failed. `message` is required and shown to the person
  (cut to 300 characters; an empty one shows as "<name> failed."). The
  optional `detail` is technical: it goes to the plugin's log, and to
  AutoScene's own log cut to 2000 characters.

Texts lose control characters and the invisible characters that hide or turn
text, and their line breaks and runs of spaces become single spaces.

### The rules

- The first `result` or `error` line counts. Later ones only go to the log.
- A plugin that exits without either line succeeded when its exit code is
  0, with no message. With another exit code it failed with "<name> stopped
  unexpectedly."; the last 20 lines of its stderr go into AutoScene's log
  beside its exit status.
- Every line that isn't a message goes to the plugin's log: text that isn't
  a JSON object, an object with a `type` this AutoScene doesn't know, or a
  message whose fields don't read. So is all of stderr. A plugin may print
  what it likes; only the messages above are taken as messages.
- A line may be at most 64 KB. A longer one is cut there, marked `[cut]` in
  the log, and never read as a message.
- A byte order mark before a line is skipped, and `\r\n` line ends are fine.
- Each run writes at most 1 MB to the plugin's log. After that, the log
  says "more output dropped", and AutoScene still reads the pipes to their
  end.

### A plugin in a few lines

```python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
import json
import sys

run = json.loads(sys.stdin.readline())
clips = run["export"]["clips"] if run["export"] else []
for i, clip in enumerate(clips):
    print(json.dumps({"type": "progress", "fraction": i / len(clips), "stage": clip["name"]}), flush=True)
print(json.dumps({"type": "result", "message": f"Saw {len(clips)} clips."}), flush=True)
```

Flush after each line. Python's output is unbuffered under AutoScene
(`PYTHONUNBUFFERED`), but another language's may wait for a full buffer, and
Activity then shows nothing until the end.

To try a plugin without AutoScene, give it a run line yourself:

```bash
echo '{"type":"run","protocol":1,"runId":"test","plugin":{"id":"dev.me.test","version":"0.1.0","dataDir":"."},"app":{"version":"1.0.3","platform":"linux-x86_64"},"tools":{"ffmpeg":"ffmpeg","ffprobe":"ffprobe","ffmpegVersion":"9.0"},"settings":{},"action":{"kind":"command","id":"hello","trigger":"palette"},"project":null,"export":null}' | uv run --script --locked main.py
```

## plugin.json (manifest version 1)

The `plugin.json` of `examples/plugins/hello-export`:

```json
{
  "manifestVersion": 1,
  "id": "dev.autoscene.hello-export",
  "version": "1.0.0",
  "name": "Hello Export",
  "description": "Writes a list of an export's clips beside them, and says hello from the command palette. An example plugin.",
  "author": { "name": "AutoScene", "url": "https://github.com/tcmmi/AutoScene" },
  "repository": "https://github.com/tcmmi/AutoScene",
  "engines": { "autoscene": ">=1.0.0, <2", "protocol": 1 },
  "runtime": {
    "type": "process",
    "command": { "*": ["uv", "run", "--script", "--locked", "main.py"] }
  },
  "permissions": ["exports:read", "exports:write"],
  "contributes": {
    "exportActions": [{ "id": "clip-list", "title": "Write a clip list", "description": "…" }],
    "commands": [{ "id": "hello", "title": "Say hello", "description": "…" }],
    "settings": [{ "key": "greeting", "type": "string", "title": "Greeting", "default": "Hello", "maxLength": 40 }]
  },
  "icon": "icon.svg"
}
```

### Fields

| Field | Required | What it is |
|---|---|---|
| `manifestVersion` | yes | `1`. |
| `id` | yes | The plugin's id (see below). It never changes: an update is a package with the same id. |
| `version` | yes | A [semver](https://semver.org) version such as `1.2.0`, at most 64 characters. |
| `name` | yes | At most 40 characters. |
| `description` | yes | At most 300 characters. |
| `author` | yes | `{ "name": …, "url": … }`: a name of at most 60 characters, and an optional `https://` link. |
| `license` | no | Such as `MIT`, at most 60 characters. |
| `repository` | no | An `https://` link. |
| `engines` | yes | `autoscene`: the AutoScene versions it works with. `protocol`: `1`. |
| `platforms` | no | The platforms it runs on. Left out or empty: everywhere it has a command for. |
| `runtime` | yes | `type`: `"process"`. `command`: the command per platform. `gpu`: `true` when it uses the graphics card (default `false`). |
| `permissions` | no | What it says it does (see below). |
| `network` | no | `{ "hosts": [...] }`: where it connects, with the `network` permission. |
| `contributes` | no | Its `exportActions`, `commands` and `settings`. |
| `icon` | no | A `.png` or `.svg` file in the plugin. |
| `$schema` | no | Allowed, and not read. |

Texts lose control characters and invisible ones. A name or title longer
than its limit is cut, with a warning. A link that isn't a plain `https://`
address (with no `user@` before its host) is left out, with a warning.

**The id** is a reverse domain name such as `dev.example.my-plugin`: 2 to 5
parts joined by dots, each 1 to 39 lowercase letters, digits and dashes,
not starting or ending with a dash. The first part can't be a name Windows
keeps for a device (`con`, `prn`, `aux`, `nul`, `com0` to `com9`, `lpt0` to
`lpt9`). The id names the plugin's folders, so choose it once.

**`engines.autoscene`** is a version requirement as Cargo writes it, with
commas between its parts: `">=1.0.0, <2"`. `">=1.0.0 <2"`, without the
comma, is refused. A pre-release of AutoScene counts as its release:
1.0.3-BETA meets `>=1.0.0` and `>=1.0.3`. A plugin made for a later
AutoScene is refused with "This plugin needs a newer AutoScene.", and one
made only for earlier versions with "This plugin only works with an older
AutoScene."

### runtime.command

`command` maps a platform to the command that starts the plugin there. Its
keys are the platforms (`windows-x86_64`, `windows-aarch64`,
`macos-aarch64`, `macos-x86_64`, `linux-x86_64`, `linux-aarch64`) and `*`,
the command for every platform without one of its own. A command is one of
two forms:

- **A uv script**: exactly `["uv", "run", "--script", "--locked",
  "<script>.py"]`. The package must hold the script and the lock `uv lock
  --script <script>.py` makes beside it, `<script>.py.lock`. `--locked`
  makes uv refuse a lock that doesn't match the script's `# /// script`
  block, so the lock decides what uv installs. On the first run uv fetches
  the packages, and a Python when the computer has none it can use; it
  keeps them in its cache.
- **A program in the plugin's folder**: `["<path>", "<argument>", …]`. The
  path is a package path (see [paths](#paths)) that doesn't start with a
  dash. On Windows the program must be an `.exe`: Windows runs a `.bat` or
  `.cmd` through `cmd.exe`. On macOS and Linux it may be a script with a
  `#!` line (but not named `.sh` or `.py`), marked to run.

A command has at most 16 parts, of at most 256 characters each, with no
control or invisible characters. AutoScene refuses:

- the computer's shells, interpreters and script hosts as the program, in
  any case and with or without `.exe`: `python`, `python3`, `pythonw` and
  the like, `uv` in any other form, `uvx`, `py`, `pyw`, `node`, `deno`,
  `bun`, `perl`, `ruby`, `php`, `osascript`, `sh`, `bash`, `zsh`, `dash`,
  `ksh`, `csh`, `tcsh`, `fish`, `env`, `wsl`, `cmd`, `powershell`, `pwsh`,
  `cscript`, `wscript`, `mshta`, `rundll32`, `regsvr32` and `msiexec`. A
  command only ever starts a file in the plugin's folder, so `python3`
  would start a file called `python3` there, not the computer's Python.
- programs whose name ends in `.bat`, `.cmd`, `.ps1`, `.sh` or `.py`, which
  a shell or an interpreter would run. Use the uv form for Python.
- absolute paths, and paths that leave the folder.

A run picks its platform's own command, else `*`. With `platforms` set, a
platform it doesn't name can't install it ("This plugin isn't made for this
computer."), and neither can one with no command. Keep in mind that `*`
covers Windows too: a `*` program that isn't an `.exe` makes the plugin
unusable there, which `asp pack` warns of.

```json
"platforms": ["windows-x86_64", "macos-aarch64", "linux-x86_64"],
"runtime": {
  "type": "process",
  "command": {
    "windows-x86_64": ["bin/frame-guard.exe", "--quiet"],
    "*": ["bin/frame-guard", "--quiet"]
  },
  "gpu": true
}
```

### permissions and network

| Permission | What the review says |
|---|---|
| `exports:read` | Reads the exported clips |
| `exports:write` | Changes or adds files beside the exported clips |
| `project:read` | Reads the project: its episodes and characters |
| `network` | Talks to the internet |

Name what the plugin does. The person sees the list before they install it,
with "Declared by the author, not enforced. The plugin runs with your user
rights." under it. An update that names a permission or a host the version
before didn't marks it "New" in its review.

A permission this AutoScene doesn't know refuses the plugin: it may stand
for more than the person agreed to.

`network.hosts` lists at most 16 lowercase host names (`discord.com`; no
scheme, no port), shown under "Talks to the internet". It is ignored, with
a warning, without the `network` permission.

### contributes

```json
"contributes": {
  "exportActions": [
    { "id": "post", "title": "Post to Discord", "description": "Posts the export's clips to the channel." }
  ],
  "commands": [
    { "id": "test", "title": "Send a test message", "needsProject": false }
  ],
  "settings": [
    { "key": "webhookUrl", "type": "string", "title": "Webhook URL" }
  ]
}
```

- **`exportActions`** (at most 8): each an entry in each export's menu, and
  a switch under "After every export" on the plugin's page. An entry is off
  while its export runs.
- **`commands`** (at most 16): each an entry in the palette's Plugins group.
  With `"needsProject": true` it is offered only inside a project, and runs
  with it; otherwise it runs with the open project, if any.
- **`settings`** (at most 32): see below.

An action's or a command's `id` is 1 to 32 lowercase letters, digits and
dashes, starting with a letter or digit, and unique among its kind. Its
`title` (required) has at most 60 characters, its `description` at most
200.

### Settings

Each setting has a `key` (a lowercase letter, then letters, digits and `_`,
at most 40 in all), a `type`, a `title` (at most 60 characters) and an
optional `description` (at most 200).

| `type` | Its own fields | Drawn as |
|---|---|---|
| `string` | `default`, `placeholder` (at most 200 characters), `maxLength` (1 to 2000, default 2000) | A text field |
| `number` | `default`, `min`, `max` (at least `min`), `step` (above 0), `integer` (default `false`) | A slider when `max` is above `min`, else a number field |
| `boolean` | `default` (default `false`) | A switch |
| `select` | `options`: 1 to 32 `{ "value": …, "label": … }`, each value unique and at most 200 characters, each label at most 60; `default`, one of the values | Up to 3 choices side by side, else a menu |

```json
{ "key": "threshold", "type": "number", "title": "Threshold", "default": 0.4, "min": 0, "max": 1, "step": 0.05 },
{ "key": "quality", "type": "select", "title": "Quality",
  "options": [{ "value": "fast", "label": "Fast" }, { "value": "best", "label": "Best" }], "default": "best" }
```

A default must fit its setting: within `min` and `max`, whole for an
`integer`, one of the options. A slider without a default starts at its
`min` (the next whole number for an `integer`), and that is its default,
which a run gets too. AutoScene checks each value the person sets
the same way, and keeps only the values that differ from their default: a
setting left alone follows the plugin's default, also after an update. A
stored value that no longer fits after an update gives way to the default.

`secret` is reserved for a later AutoScene, which won't keep such values in
plain text. This one skips a `secret` setting with the warning "secret
settings need a newer AutoScene." Until then, a token or a webhook URL can
only be a `string` setting, kept in plain text in AutoScene's `library.db`.
Say so where you ask for it (as `discord-notify` does), and never write it
to the log.

### icon

A `.png` of at most 256 KB, or an `.svg` of at most 64 KB, in the plugin's
folder. It shows in Settings, in the review and beside its export actions.
AutoScene shows it as a picture only (an SVG's scripts never run). An
`icon` that isn't a `.png` or `.svg` path is left out with a warning. One
that isn't in the package, is too large, or doesn't start as a PNG or an
SVG file does refuses the package.

### What is refused, and what is skipped

What makes the plugin unusable or could stand for more than the person
agreed to is refused, with a message for the person installing it: a
missing or wrong field, an unknown permission, a key named twice in one
object, a `plugin.json` over 256 KB. What a later AutoScene may add is
skipped with a warning, so a plugin made for one still installs in an
older one: an unknown field, contribution kind or setting type, a
platform this AutoScene doesn't know, and `pages`, which need a newer
AutoScene. A higher `manifestVersion` or `engines.protocol`, or a
`runtime.type` other than `process`, needs a newer AutoScene.

The review lists the warnings under "Skipped in its plugin.json", and so
does the plugin's page.

## Packages

A package (`.asp`) is a zip of the plugin's files and `files.lock`, which
lists each of them. Hello Export's, with two of its five files:

```json
{
  "lockVersion": 1,
  "files": [
    {
      "path": "main.py",
      "size": 4622,
      "sha256": "4b28e8dafc871cb41f00589e7df8455b244817a2fc3c67f3607ac3854ffc2a9d",
      "exec": false
    },
    {
      "path": "plugin.json",
      "size": 1681,
      "sha256": "43c6080eaa37f8fece80616c027b00e5a0cbf5897a7e841b27124cf8cb5aacd2",
      "exec": false
    }
  ]
}
```

A signed package also holds `files.lock.minisig`, the signature of
`files.lock`, and `author.pub`, the author's public key. The signature
covers the lock, and the lock covers every file.

AutoScene goes by the lock, never by what the zip says of itself. The zip
must hold exactly the listed files (folders only on their way to them),
each of its listed size and SHA-256. Whether a file may run comes from its
`exec`, never from the zip: on macOS and Linux a program unpacks as 0755,
everything else as 0644. Refused too: encrypted entries, links,
compression other than stored or deflate, data before the zip, local
headers that disagree with the zip's directory, and names that differ only
in case.

### Paths

A package path is relative, with `/` between its parts. Each part uses only
ASCII letters, digits and `. _ @ + -`, isn't `.` or `..`, doesn't end in a
dot, and isn't a name Windows keeps for a device (`NUL.txt`, `com1`).
That rules out spaces, `\`, `:` and `~`, so a path names one file on every
system. A path has at most 200 bytes and 12 parts. `files.lock`,
`files.lock.minisig` and `author.pub` are the package's own names at its
top.

### Limits

| What | At most |
|---|---|
| The package | 1 GB |
| Its files, unpacked | 2 GB together, 1 GB each |
| Entries in its zip | 4096 (so 4093 files) |
| Packed size | A file over 1 MB may be packed at most 100 times smaller than it unpacks, and so may all files together. |
| `files.lock` | 1 MB |
| `files.lock.minisig` | 4 KB |
| `author.pub` | 1 KB |
| `plugin.json` | 256 KB |

### Making a package with asp pack

```bash
asp pack <plugin-folder> <out.asp>
```

`asp pack` lists every file in the folder, but the version control folders
(`.git`, `.hg`, `.svn`) and `.DS_Store` and `Thumbs.db` at any depth, and a
`files.lock`, `files.lock.minisig` or `author.pub` at its top. It refuses
a folder that holds a link, a file a package can't name, or a secret key
(`asp.key`, or any file that is a minisign secret key), and a package
written into the plugin's own folder. It checks the manifest and the icon,
and checks the plugin for each platform it runs on as AutoScene would
there: it warns of each platform the plugin can't run on, and fails when
that is all of them. A plugin made for another AutoScene than the one asp
comes with gets a warning, not an error. It checks the package as
AutoScene would before it writes it, so a package AutoScene would refuse
is never written.

It marks a file to run (`"exec": true`) when `plugin.json` starts it on
some platform. On macOS and Linux it also marks every other file its owner
may run there, with the warning "<file> may run on this computer, so the
package marks it to run too." **On Windows a file can't be marked to run,
so a package built there marks only the programs `plugin.json` starts.** A
plugin whose program starts helper programs or scripts of its own on macOS
or Linux needs them marked too: build its package on macOS or Linux, from
a checkout that keeps the files' modes. In WSL, build it from the Linux
file system: files under `/mnt/c` usually all show as runnable.

The same folder gives the same package, byte for byte, on one system:
files go in the lock's order, dated 1980, each deflated, but those deflate
would pack more than 100 times smaller (zero-padded model weights), which
are stored.

## Trust

Who signed a package's `files.lock` decides how far AutoScene trusts it.

| Trust | Its badge | What it means |
|---|---|---|
| Reviewed | Reviewed by AutoScene | An AutoScene Plugins key, compiled into the app, signed this version. |
| Pinned | Signed by its author | Its author signed it with the key in its `author.pub`. The first install pins that key: an update must be signed with it. |
| Unreviewed | Not reviewed | Nobody signed it, or a key AutoScene doesn't know did. |
| | Loaded from a folder | A developer's folder, run as its files are now. It always counts as unreviewed. |

**AutoScene hasn't reviewed any plugin yet**: the app holds no review keys
so far, so every plugin is Pinned or Unreviewed for now. A key a package
brings along never makes it Reviewed.

A plugin AutoScene hasn't reviewed (Pinned, Unreviewed, or loaded from a
folder) needs **Settings → Plugins → Allow plugins that AutoScene has not
reviewed**, which is off until the person turns it on, and then the person
types its name in the review before it installs. Turning the setting on,
and each such install, also ask in a dialog of the system's own, which
names the plugin's id, version and key as AutoScene checked them: a script
in AutoScene's page can't say yes for the person. While the setting is
off, such plugins don't run, their actions aren't offered, and their runs
stop. The review can turn it on, after the same question.

Signing doesn't make a plugin safe. It makes its updates come from the same
author: a Pinned plugin takes an update only when it is signed with its
pinned key (or reviewed). Anyone else's package for that id is refused with
"This update is signed by a different author than the installed version."
Sign from the first version on.

### Updates

A package with the id of an installed plugin replaces it after its review.
The review shows the version installed, and marks the permissions and
hosts the update adds.

- **Trust:** a Reviewed plugin takes only reviewed versions. A Pinned one
  takes versions its pinned key signed, and reviewed ones. An Unreviewed
  one takes any, and a signed update pins its key.
- **Version:** an update must be newer. An older one is refused with
  "Version 1.2.0 is installed, which is newer than 1.1.0. Uninstall it
  first to go back." The same version installs again only with the same
  files, as a repair; with other files it is refused with "Version 1.2.0
  is installed already, with other files. A changed plugin needs a new
  version."
- **What the person set** stays: whether it is on, its settings, its data
  folder, and its actions that run after every export (those that still
  exist).
- **While it runs**, it can't be updated or uninstalled: "<name> is running.
  Wait for it to finish, or cancel it in Activity."

An install never leaves a plugin half done: the package is unpacked and
checked in a staging folder, checked again, and only then takes the place
of the version before. A review that waits more than 10 minutes expires:
"This plugin waited too long to be installed. Open it again."

The pin goes with an uninstall: installing the plugin again is a first
install.

## Signing with asp

```text
asp keygen <dir> [--force] [--unencrypted]
asp pack <plugin-folder> <out.asp>
asp sign <in.asp> --key <asp.key> [--out <out.asp>]
asp verify <in.asp> [--review-key <RW…>]…
```

1. **Make a key once**, outside any plugin folder:

   ```bash
   asp keygen ~/autoscene-keys
   ```

   It asks for a password, and writes `asp.key` (the secret key, locked with
   that password; an empty one is refused) and `author.pub` (the public
   key). It prints the key's id, as AutoScene shows it in the review. It
   replaces neither file unless `--force`, and refuses a folder that holds
   a `plugin.json`. `--unencrypted` writes a key without a password, for
   CI; keep that one in a CI secret.
2. **Sign each package** after `asp pack`:

   ```bash
   asp sign my-plugin-1.0.0.asp --key ~/autoscene-keys/asp.key
   ```

   It asks for the password, and adds `files.lock.minisig` and `author.pub`
   to the package (in place, or into `--out`). The signature's trusted
   comment names the plugin, its version and the lock's SHA-256:
   `autoscene-plugin;id=<id>;version=<version>;lock=<sha256>`. A signature
   is good for that one version of that one plugin only.
3. **Check a package** as AutoScene would:

   ```bash
   asp verify my-plugin-1.0.0.asp
   dev.yourname.my-plugin 1.0.0: Pinned (key 4B2D9C1E0F3A7E51)
   ```

   It prints Reviewed, Pinned with the key's id, or Unreviewed, with the
   warnings AutoScene would show. `--review-key` adds a review key to the
   app's own, to test a review signature.

The keys and the signature are minisign's formats; asp signs the prehashed
way, which is all AutoScene takes.

**Keep `asp.key` and its password safe, and keep a copy.** Without it,
nobody can sign an update that your plugin's users will take: they would
have to uninstall the plugin and install a package signed with a new key.
`asp keygen --force` warns of this when it replaces a key. A secret key in
a plugin's folder would let anyone sign as you, so `asp pack` refuses one.

## Developing a plugin

- **Load the folder** (Settings → Plugins → Load a folder…, while "Allow
  plugins that AutoScene has not reviewed" is on). AutoScene runs the
  plugin where it is, without a lock or a package: a change to a file
  counts at the next run. It reads the folder's `plugin.json` again each
  time it lists the plugins, so turning the plugin off and on shows a new
  action or setting. A uv script still needs its `.lock`.
- **The log**: right-click the plugin in Settings → Plugins, then **Show its
  log**. Its page in Settings names its folder and its log too. Each run
  starts with `--- <time> run <runId> <action>`. The plugin's lines are
  marked `[stdout]`, `[stderr]` and `[<level>]` for its log lines;
  AutoScene's own are marked `[host]`, `[result]` and `[failed]`, with the
  exit status at the end. A log over 1 MB is moved to `<id>.log.1` when the
  next run starts.
- **AutoScene's own log**, `logs/app.log` in its data folder, has the
  detail of each failed run.
- A loaded folder can't take the place of a version that AutoScene reviewed
  or that its author signed: uninstall that one first. A folder inside
  AutoScene's own `plugins` folder can't be loaded.
- Nor can a folder take the place of an installed package of the same
  version, or of a newer one. To go back to the folder after you installed
  its package to try it, uninstall the package, or raise the folder's
  version.
- Uninstalling a loaded folder makes AutoScene forget it and its settings;
  the folder stays as it is.

## Where AutoScene keeps plugins

AutoScene's data folder is `%LOCALAPPDATA%\AutoScene` on Windows,
`~/Library/Application Support/AutoScene` on macOS and
`~/.local/share/AutoScene` on Linux (`AUTOSCENE_DATA_DIR` moves it).

| Path in the data folder | What |
|---|---|
| `plugins/<id>/<version>+<8 hex>/` | An installed package's files, with its `files.lock` (and signature). |
| `plugin-data/<id>/` | The plugin's own files; each run starts there. |
| `logs/plugins/<id>.log` | Its log, and the one before in `<id>.log.1`. |
| `library.db` | Its record (`plugin.<id>`) and the settings the person changed (`plugin_settings.<id>`), in the `settings` table. |

AutoScene checks a package's files when it installs them. Afterwards it
checks `files.lock` and `plugin.json` each time it lists the plugin; one
that changed shows "Its files were changed. Install it again." Leave the
`plugins` folder to AutoScene: when it starts, it removes the plugin
folders it has no record of.

## Not in this version

- **Pages**: a plugin's own screens, in a sandboxed frame. `pages` in
  `contributes` is skipped with a warning.
- **Secret settings**: the `secret` type is skipped with a warning.
- **Other runtimes**: `runtime.type` `service` (a plugin that stays and
  answers calls) and `wasm` (a WebAssembly plugin whose permissions are
  enforced) need a newer AutoScene.
- **Calls into AutoScene**: a run gets the run line, and nothing else.
- **A catalog** to browse, review, install and update plugins in the app,
  and the review keys that come with it.
- **Shortcuts** for plugin commands.
