# Design

Numbers are targets, not promises.

## Features

- One app for everyone; anyone can create a room and invite others with a link.
- Each player records their own screen, game sound and voice, with game sound and voice kept apart.
- Recordings reach the room about a second behind while recording, so the last match can be reviewed while the next one records.
- All players' views line up automatically, with a manual nudge.
- Watch Party: everyone watches the same moment, switches between all views and one player's picture and voice, writes timestamped notes and draws.
- Game plugins mark events such as match start and end; Apex comes first.
- Nothing goes to a cloud; recordings stay on the members' PCs, with retention rules and protection against losing the last copy.

## Shape

- Windows app, the same for everyone. Tauri: a Rust core that captures, encodes and syncs, and keeps running with the window closed; a web UI.
- Rooms are kept, so recordings and notes build up over time.
- The core knows nothing about games. Game features are plugins.

## First start

The app asks a few questions once: a display name, a colour for drawings and pointers, and the language. It then creates:

- a user key, whose public key is the user ID that names the person everywhere: notes, drawings, recordings, plugin results;
- a key for this PC, whose public key names the device, with a certificate signed by the user key that links it to the user ID.

A person with a second PC, or a new one, links it to the same user ID from a PC they already use, which signs a certificate for it, so their history stays theirs. A lost PC is revoked by a record signed with the user key. The app reminds them to keep a backup of the user key. The name and colour can change at any time; the ID never does.

## Recording

- Each player records their own screen, game sound and microphone on their own PC.
- The game process is never touched: Windows Graphics Capture for the picture, per-process audio capture for game sound. Players use borderless windowed mode.
- GPU encoding. Two copies of the same capture:
  - full quality, kept by the recorder;
  - a 720p copy, shared with the room.
- H.264 and AAC in fragmented MP4, a keyframe every second, no B-frames. Game sound and voice are separate tracks.

## Data

Everything is a set of append-only files, one writer each:

- each player's recordings;
- each player's log of events: notes, drawings, match boundaries, clock offsets, plugin results, deletions.

A file never changes in the middle, so syncing is "send me the bytes after offset N". Each log record has a length, a checksum and the writer's signature; a torn last record is dropped on start. A local SQLite database is only an index and is rebuilt from the files whenever needed.

A recording is a set for the whole room: when anyone presses Record every member records their own screen, and Stop ends it for all. Each set is a folder named by its local start time, `rooms/<room>/sets/<time>/`, holding `<player-id>.mp4` for every player (own and received, streamed while they record) and `set.json`. Playback aligns the players inside a set from the start times in the logs and the measured clock offsets. A member who joins mid-set starts recording from then on; an absent member is an empty tile. Between sets, each member who has chosen a window or display and has sharing on streams a live file, `rooms/<room>/live/<player-id>-<start ms>.mp4`, started anew at every set boundary and after a size limit; everyone keeps only the newest live file per player. Nothing is captured until a source is chosen.

Discovery: the default n0 address lookup publishes only the relay URL, not IP addresses. Anyone who dials an ID can still see its IP while hole punching, so IDs and invite codes stay inside the team.

Record kinds in a log (`rec` and `rec_end` carry the set and room, notes carry the set): `rec` (a recording starts: file name, start time, source kind and name, capture size, fps, encoder and codec, game sound process, microphone, app version; tracks are video, game sound, microphone), `rec_end`, `note`, `note_edit`, `note_del`, `note_restore` (edits and deletions refer to a note by its id), `offset` (clock offset to a peer), `layout` (a viewer's arrangement of tiles). Unknown kinds are kept and ignored.

Edits and deletions are new records. Only the recorder can delete a recording for everyone, and it goes to a trash period first. The app never removes the last copy of anything it knows about.

## Sharing

- Peers connect directly over the internet with hole punching, and through a relay when they cannot. Devices are identified by their public keys and prove them when connecting; a room only accepts devices with a certificate from one of its members. Invite links carry the room ID, the creator's device key and a one-time token. Relay and discovery servers see nothing. iroh is the first candidate.
- The 720p copy and the logs stream to the room as they are written, so others see a match about a second behind.
- Peers pass on what they have received, so the recorder uploads each piece once and rooms of 4 or more still work.
- Full quality is fetched only when someone asks for it.
- Retention: full quality for a few weeks, the 720p copy for a few months, matches with notes kept.

## Time

- Peers measure clock offsets continuously (NTP style, lowest-delay of the last few samples) and log them.
- Playback lines up POVs from those offsets. Game events from plugins refine the alignment, and a manual nudge is always possible.

## Watch Party

Each peer plays its own local copy; only small control messages travel.

- One click to join from a recording. Anyone can play, pause, seek and switch views; the screen says who did what, and the last seek can be undone.
- 0 shows all views; 1 to N show one player's picture and voice. The view is shared, volume is personal.
- Anyone can start presenting; others are pulled in after a short grace period and can step out of sync and snap back.
- Commands carry a sequence number and a shared start time slightly in the future. One peer (the leader) orders them; if it leaves, the next one takes over.
- Drift: under 40 ms nothing, up to 400 ms adjust speed by up to 5%, beyond that seek. Pause and seek wait at most 2 s for everyone; a slow peer catches up alone instead of holding the room.
- Notes are timestamped and do not pause others. Drawing: pen, arrow and a fading laser pointer, tied to the recording, view and frame time. Live strokes are broadcast; finished ones go to the log.

## Plugins

- Local plugins run on each recorder's own picture and sound while recording, for example Apex match start and end, knocks and ring closes. Each player handles only their own stream, so nothing runs twice.
- Team plugins need everyone's recordings and run once, on the leader. Results record the plugin, its version and its inputs, so a duplicate run is harmless.
- Plugins can be turned on and off: local ones per device, team ones per room.
- Plugins only see captured picture and sound.

## Leader

One member coordinates: Watch Party ordering, team plugins and alignment. It defaults to the room's creator, falls back to another online member by a fixed rule, and holds no data the others lack. An always-on member, such as a home server running the app without a window, makes a good leader and long-term keeper.

## Accounts

The app is free and open source and works without any account. Accounts can come later as a layer that links a user ID to an account; the data model does not change. Recordings are never sold or put behind a paywall; EA's terms forbid that for its games.

## Later

- A Media Source player for live viewing, instead of reloading the growing file.
- A local SQLite index over the logs for lists and search, rebuilt from the logs.
- Per-author sequence numbers in the logs and relaying of logs between members.
- Game plugins that mark events such as match start and end.
