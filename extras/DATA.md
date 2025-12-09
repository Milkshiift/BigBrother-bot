PLACEHOLDER

Big Brother stores data in a structured file system designed for easy parsing by downstream tools (like data analysis scripts or viewers).

```text
./data
├── downloads.ndjson  # Internal tracker for asset download states
└── {GUILD_ID}
    ├── metadata
    │   ├── members.ndjson  # Member joins, leaves, and profile updates
    │   ├── roles.ndjson  # Role creations, edits, colors, perms
    │   ├── channels.ndjson  # Channel names, topics
    │   └── ...
    ├── messages
    │   ├── {CHANNEL_ID}.ndjson  # Full message log for specific channels
    │   └── ...
    └── assets  # Guild assets
        ├── icons
        ├── avatars
        ├── stickers
        └── ...
```

### The NDJSON Format
Every file is a stream of JSON objects separated by newlines. This allows files to be read efficiently line-by-line without loading the entire dataset into RAM.

**Example `messages/12345.ndjson` entry:**
```json
{"t":"c","ts":1715421000000,"i":987654321,"ct":"Hello world!","a":123456...}
```
*   `t`: Type (`c`=Create, `u`=Update, `d`=Delete)
*   `ts`: Timestamp (Unix ms)
*   `i`: Message ID
*   `ct`: Content