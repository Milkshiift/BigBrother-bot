<h1 align="center">BigBrother 👁️</h1>
<div align="center">
<a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-lang-000000.svg?style=flat&logo=rust" alt="Rust lang" /></a>
<a href="https://github.com/twilight-rs/twilight"><img src="https://badgen.net/static/built%20with/twilight/cyan?icon=discord" alt="Built with twilight" /></a>
<a href="https://www.gnu.org/licenses/agpl-3.0"><img src="https://img.shields.io/badge/license-AGPL%20v3-green" alt="AGPL-3.0" /></a>
</div>
<br>

**BigBrother** is a continuous archiver for Discord servers. Instead of taking snapshots, it tracks the complete history of all server changes, creating a comprehensive local mirror that:
1. 📜 Preserves the **full audit trail** of every modification, not just the current state.
2. 🔁 Stays **continuously synchronized** with your server in real-time.
3. 🔤 Is fully **plain-text** (NDJSON), and not behind a database.

<div align="center">
<img align="center" src="extras/bbDemo.webp" alt="Demo browsing a downloaded server using Yazi">
</div>

## ✨ Key Features

*   **⏪ Catchup**: Automatically fetches unsynced data from bot downtime.
*   **💾 Metadata Log**: Saves every change to channels, members, roles, emojis, stickers, and the guild.
*   **🖼️ Asset Mirroring**: Downloads attachments, avatars, emojis, stickers, and guild icons/banners in full quality.
*   **📄 Append Log**: Data is *never* deleted or overwritten.
*   **⚡ High Performance**: Your internet connection and storage I/O are the bottleneck, never the bot.
*   **🛡️ Reliable**: Built like failure is not an option. Data continuity is sacred.

## 🚀 Getting Started

### Prerequisites

*   [Rust](https://rust-lang.org/learn/get-started/) 
*   A [Discord Bot](https://discord.com/developers/applications) with:
    * Server Members Intent
    * Message Content Intent
    * "View Channels" and "Read Message History" permissions on the server.
    * If asset downloads don't work for you, you need to enable the Administrator permission. This is a Discord quirk, I couldn't figure out a way to bypass this.

### Installation
#### Manual
```bash
git clone https://github.com/Milkshiift/BigBrother-bot.git
cd BigBrother-bot
cargo run --release
```
In case you need it, you can find the resulting binary at `./target/release/BigBrother`
#### Nix
Add flake to inputs:
```nix
bigbrother.url = "github:Milkshiift/BigBrother-bot";
```
Enable service:
```nix
services.bigbrother = {
    enable = true;
    token = "your_bot_token";
    # Example config
    # settings.storage.autoflushInterval = 60000;
};
```

### Configuration

Big Brother is configured via `config.toml` or Environment Variables.

1.  Set your bot token with:
    * The `discord_token` parameter in `config.toml`.
    * Or the environment variable (recommended):
        ```bash
        export BIGBROTHER_DISCORD_TOKEN="your_bot_token_here"
        ```

2. Set the `data_path` in `config.toml`. This is the location where the bot will store all the data.

3.  Start the bot with the same command.   

You can find option descriptions [here](https://github.com/Milkshiift/BigBrother-bot/blob/main/src/settings.rs#L28).    
Config changes will not take effect until restart.

## 🗄️ Data Storage
See [DATA.md](https://github.com/Milkshiift/BigBrother-bot/blob/main/extras/DATA.md)

## 📖 Background & FAQ
### Why I built this
I created BigBrother for a server I share with friends that has evolved through quite a few "themes" (unified sets of server names, roles, member nicknames, etc.).
This history to me is just as valuable as the message content itself, but there was previously no way to capture it, until now.

### Why is there no development history?
The development of this project took place in a private repository over the course of ~1.5 years with more than 200 commits.
As I initially did not intend for it to be public, the commit history has a bunch of tokens and other private information I would not like to share, so I created a new repository for a public release.
