# Discord Notify

An example AutoScene plugin that talks to the internet: it posts an
export's name and clips to a Discord channel through the channel's webhook.
[docs/PLUGINS.md](../../../docs/PLUGINS.md) explains each part.

- **Post to Discord** (each export's menu, or after every export): posts
  the export's name, preset and how many clips it made, and, with **List
  the clips** on, each clip's name and episode. It never posts a file, a
  path or a frame.
- **Send a test message to Discord** (the command palette): posts "AutoScene
  can post here." to check the webhook.

## The webhook URL is stored in plain text

Anyone who has a webhook's URL can post to its channel. Settings of the
type `secret` are reserved for a later AutoScene, which won't keep them in
plain text; this one skips such a setting with a warning. So the URL is a
plain `string` setting: AutoScene keeps it in plain text in its
`library.db`, and hands it to each run. The script never writes it to its
log or its messages.

If the URL gets out, delete the webhook in Discord (the channel's settings,
Integrations, Webhooks) and make a new one.

## Where it connects

`plugin.json` says it uses the network and names Discord's hosts. AutoScene
shows them before install, but doesn't enforce them, so the script refuses
any URL that isn't an `https://` webhook on one of them. It checks
certificates against the system's own, through
[truststore](https://pypi.org/project/truststore/), the one dependency
`main.py.lock` pins.

## Try it

You need [uv](https://docs.astral.sh/uv/) and the `asp` tool:

```bash
cargo install --path crates/as-plugin --features cli --bin asp
asp pack examples/plugins/discord-notify discord-notify.asp
```

Turn on **Settings → Plugins → Allow plugins that AutoScene has not
reviewed**, then **Install a plugin…** and pick `discord-notify.asp`. Paste
the webhook URL on its page in Settings, and run **Send a test message to
Discord** from the palette. Its page also has **After every export**, to
post each export once it finishes.

The first run can take a while: uv may need to fetch a Python and
truststore.
