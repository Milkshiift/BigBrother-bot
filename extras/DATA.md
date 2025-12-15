## General Philosophy
- All data except assets and attachments is stored in an append-only log in the [Newline Delimited Json](https://github.com/ndjson/ndjson-spec) format.
- As the logs are only appended to, data is never deleted or modified. All changes are new log entries ("events").
- For compactness, log entries have minified key names.
- The file system serves as a database, where IDs are usually chosen as unique identifiers for file names.

## File system
```text
./data
├── downloads.ndjson  # Internal tracker for asset download states
└── {guild_id}
    ├── metadata
    │   ├── members.ndjson  # Member joins, leaves, and profile updates
    │   ├── roles.ndjson  # Role creations, edits, colors, permissions
    │   ├── channels.ndjson  # Channel names, topics
    │   ├── guild.ndjson  # Server name, icon hash, etc
    │   └── ...
    ├── messages
    │   ├── {channel_id}  # Folder containing channel attachments
    │   │   └── {attachment_id}_{attachment_file_name}.{ext}
    │   ├── {channel_id}.ndjson  # Full message log of a channel
    │   └── ...
    └── assets  # Guild assets
        ├── avatars
        │   └── {user_id}_{hash}.{ext}
        ├── emojis
        │   └── {emoji_id}.{ext}
        ├── icons
        │   └── {hash}.{ext}
        └── stickers
            └── {sticker_id}.{ext}
```

## "Catchup"
Catchup is the process of fetching unsaved history. It runs first-thing on every launch.    
It will fetch full history if there is none (first launch), or partial history to fill in downtime.    
Catchup saves messages, metadata and assets.

## Message storage
Every channel and thread has its own `.ndjson` file (`messages/{CHANNEL_ID}.ndjson`).

### Schema
Each line is a JSON object representing an event. The type of event is determined by the `t` field.    
You can see the exact up-to-date definitions in [messages.rs](https://github.com/Milkshiift/BigBrother-bot/blob/main/src/messages.rs).

#### Event Types (`t`)
| Value | Description           | Fields                                                          |
|-------|-----------------------|-----------------------------------------------------------------|
| `c`   | Create Message        | [Message Object](#message-object)                               |
| `u`   | Update Message        | [Message Object](#message-object)                               |
| `d`   | Delete Message        | `i` (Msg ID)                                                    |
| `bd`  | Bulk Delete           | `is` (Array of IDs)                                             |
| `ra`  | Reaction Add          | `i` (Msg ID), `u` (User ID), `e` ([Reaction](#reaction-object)) |
| `rr`  | Reaction Remove       | `i` (Msg ID), `u` (User ID), `e` ([Reaction](#reaction-object)) |
| `rra` | Reaction Remove All   | `i` (Msg ID)                                                    |
| `rre` | Reaction Remove Emoji | `i` (Msg ID), `e` ([Reaction](#reaction-object))                |

#### Message Object
Used in `Create` (`c`) and `Update` (`u`) events.

| Key  | Type   | Description                                                                                                                       |
|------|--------|-----------------------------------------------------------------------------------------------------------------------------------|
| `i`  | u64    | Message ID                                                                                                                        |
| `ct` | string | Content                                                                                                                           |
| `ca` | u64    | Created At (Unix millis)                                                                                                          |
| `ea` | u64    | Edited At (Unix millis)                                                                                                           |
| `a`  | u64    | Author ID                                                                                                                         |
| `e`  | array  | Embeds ([Twilight Embed Structure](https://docs.rs/twilight-model/0.17.1/twilight_model/channel/message/embed/struct.Embed.html)) |
| `at` | array  | Attachments (List of u64 IDs)                                                                                                     |
| `s`  | array  | Stickers (List of u64 IDs)                                                                                                        |
| `r`  | array  | Reactions (List of `[ReactionData, count]`)                                                                                       |
| `ri` | u64    | Reference Message ID (Reply)                                                                                                      |

#### Reaction Object
| Key | Type   | Description          |
|-----|--------|----------------------|
| `c` | u64    | Custom Emoji ID      |
| `u` | string | Unicode Emoji String |
*(Only one of `c` or `u` will be present)*

## Metadata storage
Metadata updates are stored in specific `.ndjson` files within the `metadata/` directory.

### Members (`metadata/members.ndjson`)
| Key  | Type    | Description                                  |
|------|---------|----------------------------------------------|
| `i`  | u64     | User ID                                      |
| `u`  | string  | Username                                     |
| `gn` | string? | Global Display Name                          |
| `a`  | string? | Avatar Hash                                  |
| `j`  | u64?    | Joined At (Unix millis)                      |
| `l`  | u64?    | Left At (Unix millis) - Present if user left |
| `r`  | array   | Roles (List of u64 Role IDs)                 |
| `nk` | string? | Guild Nickname                               |
| `b`  | bool    | Is Bot                                       |

### Roles (`metadata/roles.ndjson`)
| Key  | Type   | Description                    |
|------|--------|--------------------------------|
| `i`  | u64    | Role ID                        |
| `n`  | string | Name                           |
| `c`  | u32    | Color (Integer representation) |
| `p`  | i64    | Position                       |
| `ps` | string | Permissions (Bitfield string)  |
| `h`  | bool   | Hoist (Display separately)     |
| `m`  | bool   | Mentionable                    |
| `d`  | bool   | Deleted                        |

### Channels (`metadata/channels.ndjson`)
| Key  | Type    | Description                                                                                |
|------|---------|--------------------------------------------------------------------------------------------|
| `i`  | u64     | Channel ID                                                                                 |
| `n`  | string  | Name                                                                                       |
| `t`  | string? | Topic                                                                                      |
| `ty` | u8      | [Type](https://docs.rs/twilight-model/0.17.1/twilight_model/channel/enum.ChannelType.html) |
| `p`  | i32     | Position                                                                                   |
| `pi` | u64?    | Parent ID (Category)                                                                       |
| `ns` | bool    | NSFW                                                                                       |
| `d`  | bool    | Deleted                                                                                    |

### Guild (`metadata/guild.ndjson`)
| Key  | Type    | Description |
|------|---------|-------------|
| `n`  | string  | Name        |
| `ic` | string? | Icon Hash   |
| `bn` | string? | Banner Hash |
| `d`  | string? | Description |
| `s`  | string? | Splash Hash |

### Emojis (`metadata/emojis.ndjson`)
| Key | Type   | Description |
|-----|--------|-------------|
| `i` | u64    | Emoji ID    |
| `n` | string | Name        |
| `a` | bool   | Animated    |
| `d` | bool   | Deleted     |

### Stickers (`metadata/stickers.ndjson`)
| Key | Type   | Description                                                                                                             |
|-----|--------|-------------------------------------------------------------------------------------------------------------------------|
| `i` | u64    | Sticker ID                                                                                                              |
| `n` | string | Name                                                                                                                    |
| `f` | u8     | [Format Type](https://docs.rs/twilight-model/0.17.1/twilight_model/channel/message/sticker/enum.StickerFormatType.html) |
| `d` | bool   | Deleted                                                                                                                 |