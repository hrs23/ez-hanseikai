# Storage format

What a program needs to read, or add to, the recordings on one PC. How members talk to each other is in [the protocol](protocol.md); what the app does is in [the design](design.md).

## Principle

Store raw, compute views later.

- Files and logs hold only what was captured or done, as it happened: append-only records, fMP4 segments exactly as encoded, voice continuous for the whole recording.
- Everything shown is computed when read: sets, parts, start and end, clock alignment, the current text of an edited note, deletions, the layout, room names, membership, match chapters, the replay of a debrief.
- Stored data is never rewritten or post-processed. The only repairs are at start: the member's own open `.part` is cut back to its last whole fragment, and a torn tail of a room log or of `pending.bin` is cut back to its last whole record.
- An edit or a deletion is a new record.
- Logs are not signed. Authenticity comes from the connection: it is encrypted, authenticated to member IDs, and refused to non-members. Any member may serve any member's log, and members trust each other.

## Data folder

`%LOCALAPPDATA%\dev.ez-hanseikai.app`, or the folder in the environment variable `EZH_DIR`. A reader needs only these:

| Path | Content |
| --- | --- |
| `key.bin` | 32 raw bytes: the Ed25519 secret key. Its public key, in iroh's text form, is the member ID. |
| `rooms.json` | `{"active": <room id or null>, "list": [{"id", "name", "members": [member ID], "names": {member ID: name}, "icon", "invite": [member ID, token], "removed": [member ID]}]}`. `icon` and `invite` are optional; `icon` is a cache of the room icon. |
| `app.lock` | Held exclusively while the app runs. A tool writes only when it can open this file for writing. |
| `rooms/` | One folder per room. |

Settings, the app's logs, downloads and other files are not part of the format.

## IDs

- Member ID: the iroh `EndpointId` text. The short ID is its first 10 characters (used in `offset.peer`).
- Room ID: 8 hex characters for rooms made by the app; any `valid_id` is read (at most 40 characters of letters, digits, `-` and space, no space at either end).
- Set ID (a recording): `YYYY-MM-DD HH-MM-SS`, the starter's local time; any `valid_id` is read.
- Capture ID: digits, at most 20 (the capture's start in milliseconds). A capture is one encoder run.
- Series: `v` (picture plus app sound) and `m` (voice).

## Room folder

```
rooms/<room>/
  logs/<member id>.bin
  pending.bin
  recordings/<set>/<member id>/<capture id>/<series>/init.mp4
  recordings/<set>/<member id>/<capture id>/<series>/000001.m4s
  standby/<member id>/<capture id>/<series>/...
  thumbs/<set>.jpg
```

- A series is fMP4: `init.mp4` and numbered segments (`000001.m4s`, `000002.m4s`, ...) of four one-second fragments each. `init.mp4` followed by the segments 1..n, concatenated in order, is a playable file. Codecs are H.264, HEVC or AV1, and AAC.
- A name ending in `.part` is being written or received; a program ignores it.
- `standby/` holds live-only segments that belong to no recording, and is emptied at every start.
- `pending.bin` is a log of the member's own records that wait until the member's own log is restored from the room; it has the log format.

## Log files

`logs/<member id>.bin` is the log of one member. Only that member appends to it; every other copy is a byte prefix of it.

A record is `u32 LE length`, `u32 LE CRC32 (IEEE) of the body`, then the body: a JSON object. A reader stops at the first record that is short, fails its CRC or is not JSON; the app cuts such a tail at start.

- Unknown kinds and unknown fields are ignored. A record with a field of the wrong type is skipped.
- `t_us` is a time in microseconds since the Unix epoch on the writer's corrected clock. A reader subtracts the log's clock offset (below) to get its own clock.
- A set's media for a record lies at `recordings/<set>/<member id>/<capture>/<series>/`, named by `seg`.

## Records

Every kind the app writes. The body's field `k` holds the kind, for example {`k`: "rec", `room`: ...}. Fields marked `?` are optional.

| Kind | Fields | Meaning |
| --- | --- | --- |
| `rec` | `room`, `set`, `start_us`, `by` (name of the starter), `voice`, `of?` | The writer joins the set: its part begins. `of` names the recording a debrief is about. Capture details (`source_kind`, `source`, `width`, `height`, `fps`, `kbps`, `encoder`, `codec`, `audio`, `mic`, `version`) are copied in. A recording made elsewhere also has `imported`, `view` and `title?` (below). |
| `rec_end` | `set`, `room`, `t_us`, `stop` | The writer's part ends. `stop` is true when someone pressed Stop, so a member who missed it ends the set when it finds this record in any log. |
| `cap` | `room`, `capture`, `start_us`, `init` {`v`, `m`: {`size`, `sha256`}}, plus the capture details of `rec` | A capture starts or an init arrives. All `cap` records of one capture are merged, later fields winning. |
| `seg` | `capture`, `series`, `n`, `set`, `t_us`, `d_us`, `size`, `sha256` | A finished segment of a set: number `n` (from 1), its start, duration, size and SHA-256. Only segments of a set are logged. |
| `event` | `id`, `t_us`, `kind`, `target`, `text`, `values`, `tags`, `source?` | A note, mark, drawing, board card or plugin result. `source` is `ai` for a note written through the local API. `id` is unique per author. `target` is `{"t_us"}` for a moment or `{"from_us", "to_us"}` for a range, with `set?` and `file?` (a member ID). A plugin result also has `plugin` {`id`, `name`, `version`}, `job` and `run`. |
| `event_edit` | `id`, `t_us`, `text`, `tags` | The new text of the writer's event. |
| `event_del` | `id`, `t_us` | The writer's event is deleted. |
| `event_restore` | `id`, `t_us` | The deletion is undone. |
| `set_del` | `room`, `set`, `t_us` | Every member drops the set. |
| `remove` | `member`, `t_us` | The member is removed from the room. |
| `admit` | `member`, `t_us` | The member is let in (again). |
| `room_name` | `name`, `t_us` | The room is renamed. |
| `room_icon` | `icon`, `t_us` | The room's icon: an image data URL (PNG, JPEG or WebP, base64, at most 16384 characters), or "" for none. |
| `profile` | `color?`, `avatar?`, `t_us` | The writer's colour (0 to 7, an index into the member palette) and avatar (an image data URL as above, or a built-in name of 1 to 16 lowercase letters). A full snapshot; a missing field is none. |
| `offset` | `peer` (short ID), `t_us`, `rtt_us`, `offset_us` | The writer measured the peer's clock: `offset_us` is theirs minus the writer's. |
| `layout` | `set`, `t_us`, `layout` | The writer's layout for the set; `set` is also inside `layout`. A `layout` record without `set`, written while watching, belongs to no set's default layout and is ignored. |
| `plugin` | `plugin`, `on`, `version`, `runner?`, `t_us` | The room's choice for a plugin. `runner` is the member who runs it; without it, the writer. Older versions also wrote `settings`, which is ignored. |
| `job` | `state`, `job`, `run`, `t_us`, `plugin?`, `version?`, `file?`, `error?`, `events?`, `tags?` | A step of an analysis job: `state` is `claim`, `fail`, `done`, `retry` or `stop` (`stop`, and `yield` from older versions, read as waiting). |
| `party` | `set`, `t_us`, `shared` | The Watch Party state while a set records. |
| `ai_del` | `set`, `t_us` | Hides every note with `source` `ai` of the set written before it, from any log. |
| `file_bad` | `room`, `set`, `file`, `size`, `t_us` | A fetched file failed its check and was deleted (`file` is `<member id>/<capture>/<name>`). |

How readers derive things:

- A set exists for each `rec` whose set ID is valid and not dropped by a `set_del`. A member has one part per set. The part is that member's `rec`, `seg` and `rec_end` records, and its captures are the `cap` records of the captures its segments name. A later `rec` after a `rec_end` reopens the part. A repeated `seg` for the same capture, series and `n` is ignored. `n` is 1 to 4294967295, `series` is `v` or `m`, `capture` is a capture ID.
- The set starts at the earliest start of any part, and ends at the latest end. A part with no `rec_end` is open; a set is recording while a part is open and present, and a `rec_end` with `stop` true by anyone ends the set.
- Clock offset: the writer's own log has offset 0. Another log's offset is the newest `offset` measurement of that member by the reader, or its live measurement; a recording made elsewhere takes the offset of the member named in `imported`. A record's time on the reader's clock is `t_us` minus the offset.
- Events: for each author and `id`, the first `event` counts; later `event_edit`, `event_del` and `event_restore` records of the same author change its text, tags and deletion. A plugin result cannot be edited, and is shown only while its plugin is on, for the newest run of that plugin and file that has a `job` record with state `done`, whatever its version or settings (the lowest member ID and run on a tie). A `board` event is a card: `values` {`card`, `title?`, `pos?`, `gone?`}, the newest event of a card wins, and an event without `card` is the team card.
- The room name is the newest `room_name` by corrected time, and the room icon the newest `room_icon` the same way. A member's colour and avatar are those of the newest valid `profile` in their own log, by file order; a record with a wrong field is skipped. The colour of a member, unless they chose one, comes from a hash of their ID: everyone in the room (removed members too) is taken in order of hashed colour, then ID, and each gets the first free palette index from their hash upward, wrapping around, or the hashed colour itself if all eight are taken; a choice replaces only that member's colour, and two members may share one. A built-in name the reader does not know shows the initial. The newest `plugin` record per plugin wins. The latest `remove` or `admit` per member wins: the member is removed or in. The `layout` of a set is the newest one for it. The state of a plugin on a file is its newest `job` record over every key and member. `worker` records of older versions are ignored.
- A debrief is a voice-only set whose `rec` names a recording in `of`; an older one without `of` links to the `shared.set` of its first `party` record. Its `party` records are what the Watch Party showed while it was recorded.

## Placing a recording made elsewhere

A tool can add a recording that was made by other software, such as the views of a game recorded by someone outside the room. The recording appears as a set with one made-up member per view.

1. Close the app; a tool writes only while it can open `app.lock` for writing.
2. Make a new Ed25519 key per view and throw the secret away. Its public key, in iroh's text form, is the view's member ID.
3. Write `rooms/<room>/logs/<view id>.bin` with these records in order:
   - `rec` {`k`: "rec", `room`, `set`, `start_us`, `by`: a display name, `voice`: false, `imported`: your own member ID, `view`: the view's name, `title?`}
   - `cap` {`k`: "cap", `room`, `capture`, `start_us`, `init`: {`v` or `m`: {`size`, `sha256`}}, `source_kind`: "import", `width`, `height`, `kbps`, `codec`, `version`}
   - one `seg` (with `k`: "seg") per segment
   - `rec_end` {`k`: "rec_end", `set`, `room`, `t_us`, `stop`: false}
4. Put the media at `recordings/<set>/<view id>/<capture>/{v,m}/init.mp4` and `NNNNNN.m4s`. The sizes and SHA-256 must match the records.
5. Append `remove` {`k`: "remove", `member`: the view ID, `t_us`} to your own log, or to `pending.bin` when that file exists (your own log is then being restored, and the app appends to `pending.bin`; writing the log itself would fork it). In your entry in `rooms.json`, add the view ID to `removed` and its name to `names`.

Members get the view like any other: a removed member's log is pulled and shared, nobody dials it, and it is not listed as a member. The reader takes the view's name, title and importer from its first `rec`.

## Files of older versions

The app starts and works with files of older versions present, and reads none of them. These shapes are old, judged by the name at each depth:

- In the data folder: `log.bin`, `peer/`, `peer.txt`, `peers.txt`, `running.marker`, `plugins/` (tools and models of older versions), and `rec-<digits>.mp4`.
- In `rooms/<room>/`: a folder named by a member ID, `sets/` and `live/`.
- In `logs/`: `<name>.bin` where the name is not a member ID.
- In a capture folder `recordings/<set>/<member id>/<capture id>/`: `verified.json` and `cap.json`.
- In a series folder: `open.json`.

Records of the older kinds `file` and `note` stay in current logs and are ignored; logs are append-only and members sync them by byte offset. `rooms/solo/`, where versions before 0.19 recorded outside a room, is no longer opened and stays like any unlisted room.

Settings > Recordings > Delete old files removes exactly these paths, after one confirmation, and nothing else: unknown names, `key.bin.old` and the like, orphan sets and unlisted rooms stay. Nothing deletes them on its own.

## Export folder

Recording > Export to folder… writes a new folder of plain files for a video editor. Every file starts at the recording's start (a late part is padded with black and silence) and runs to its end, so the files line up with no manual syncing.

- `<name>.mp4`: a member's picture and app sound. A member who has only voice gets `<name>.m4a`. `<name>` is the member's name made safe for file names and unique.
- `<name> voice.m4a`: that member's voice, when there is also a picture.
- `notes.csv`: notes and marks, one row each, with the columns `seconds,kind,text,tags` (seconds from the start; `kind` is `note` or `mark`; `tags` are separated by spaces). Written only when there are any.
- `chapters.vtt`: chapters as WebVTT cues (chapters people made, and the matches that the Apex matches plugin found). Written only when there are any.
- `<name>.vtt`: the spoken lines of that member as WebVTT cues, from the Transcription of older versions.
- `watch.json`: for a debrief, its `party` records as `[{"t_us", "by", "set", "at_us", "playing", "rate"}]`, in microseconds on the exporting PC's clock.
