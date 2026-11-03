# Hello Export

An example AutoScene plugin: a Python script that uv runs, with an export
action, a palette command and three settings. [docs/PLUGINS.md](../../../docs/PLUGINS.md)
explains each part.

- **Write a clip list** (each export's menu, or after every export): writes
  `clips.txt`, or `clips.md` with the Markdown format, into the export's
  folder. It lists each clip with its episode and its first and last
  frame's times. A file of that name there is replaced.
- **Say hello** (the command palette): answers with the AutoScene, platform
  and FFmpeg the run got.

## Try it

You need [uv](https://docs.astral.sh/uv/) and the `asp` tool:

```bash
cargo install --path crates/as-plugin --features cli --bin asp
asp pack examples/plugins/hello-export hello-export.asp
```

AutoScene hasn't reviewed this package, so turn on **Settings → Plugins →
Allow plugins that AutoScene has not reviewed** first. Then **Install a
plugin…** and pick `hello-export.asp`, or **Load a folder…** and pick this
folder to run it from here while you change it.

The first run can take a while: uv may need to fetch a Python.

## Change it

`main.py.lock` is made by `uv lock --script main.py`. Make it again after
you change the `# /// script` block at the top of `main.py` (its Python or
its dependencies), or uv refuses to run the script.
