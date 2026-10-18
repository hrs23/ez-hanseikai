# Design

What the app does and how, as built and as decided. Items marked (planned) are decided but not built yet; they are also listed under [Planned](#planned).

## Shape

- A Windows app, the same for everyone. Tauri: a Rust core that captures, encodes and syncs, and keeps running in the tray with the window closed; a web UI.
- Two tabs, Record and Player, a native menu bar and a tray icon. Each action and piece of information has one place, with no explanatory text.
- Everyday review features live in the app: recording, sharing, rooms, playback, notes, drawing, Watch Party, export and showing events. Game-specific analysis lives in plugins. The UI says "app" for capture sources and their sound (planned; it says "game sound" now); game terms such as matches come from plugins as tags and chapters.
- Nothing goes to a cloud. Everyone is equal: no host and no leader for recording or playback.
- English and Japanese, following the Windows language at first and switchable in the settings (planned; English only now).

## Identity

One key per person, made on first start; its public half is the person's ID everywhere. The name can change; the ID cannot. Moving to a new PC means exporting and importing the key (File menu).

## Rooms

- Without a room, the Record tab shows Create room, Join with a code and recent rooms. The top bar shows the room name and who is online; the room menu switches, invites, lists members and leaves.
- Invite codes work once, only while the inviter's app runs, and can be cancelled. Any member can invite, and rename the room (planned). Joined rooms stay in the list; one is open at a time.
- Leaving a room removes it from this PC only.

## Live and recording

- The source is a window or a whole display, chosen per person; it starts as none, and nothing is captured or sent until one is chosen. Others then see a placeholder tile with the name. While a whole display is shared, the top bar says so.
- The microphone is on by default, as its own track, and can be muted. App sound comes from per-process capture. Each source has a dB meter with peak and clipping and a volume.
- Inside a room everyone's picture and sound stream all the time and show live in the Record tab, a few seconds behind. Sharing off sends nothing; one speed limit caps the upload.
- One recording per room: anyone's Record starts it for every member and anyone's Stop ends it. That span is a set holding everyone's file, and every member sees the same list. Nothing is kept while nobody records. A member who joins mid-set records from then on; an absent member is an empty tile.
- One ffmpeg process per person encodes the capture once (scaled to the shared width) and writes, through the tee muxer, short live segments that stream to the room and, while the room records, the set file. Only the newest few live segments are kept.
- Codec: AV1, then HEVC, then H.264, the best one that every member can play. Fragmented MP4, a keyframe every second, no B-frames; app sound and voice are separate tracks. There is no separate full-quality copy.

## Data

- Each person writes their own append-only files: their recordings and their log. The logs are the source of truth; a torn last record is dropped on start.
- A set is a folder, `rooms/<room>/sets/<local start time>/`, with `<member-id>.mp4` for every member and `set.json`, which keeps the recorder's tile order and layout as the set's default.
- Log record kinds include `rec`, `rec_end`, `file`, `file_bad`, `note`, `note_edit`, `note_del`, `note_restore`, `offset` and `layout`. Unknown kinds are kept and ignored. Edits and deletions are new records.

## Sharing

- Peers connect directly with iroh, through a relay when they must. Connections are encrypted and accepted only from room members. Address lookup publishes only the relay URL.
- On connecting, members pull every member's log from any peer that has it, so late or offline members learn every set. Missing files then come from any member with a verified copy, resuming where they stopped, behind live streams and within the speed limit.

## Verification

When a set ends, each recorder logs the size and SHA-256 of its file. Every member hashes its copies once complete; a copy that does not match is logged, deleted and fetched again from a good copy. Members tell each other which files they hold, so the set list shows whether everyone has everything or what is missing.

## Time

Peers measure clock offsets continuously and log them. Playback lines up everyone in a set from the recording start times and the offsets, with a manual nudge.

## Player and Watch Party

- Each person plays their own copy. Views: all players, or one player's picture and sound (keys 0 and 1 to 6).
- Layouts: auto grid, one large with the others beside or below, and a row; for 1 to 6 players. The recorder's layout is the set's default; each viewer can rearrange.
- Watch Party: those who turn it on follow each other's play, pause, seek and view. The newest command wins and the current state is repeated, so anyone who joins catches up; no member leads. The screen says who did what, and the last seek can be undone.
- Volume is personal. Each person can set app sound and voice volume and mute for each member for themselves (planned; one volume now).

## Events

Notes, markers and plugin results are one kind of record, an event: who made it (person or plugin), its kind, an optional target (a moment, a range, a file or the whole set) and its content (text or numbers), with tags (people write `#tag` in a note; plugins tag their own). Events can be filtered and searched by tag. Only people's events can be edited or deleted. Notes are built: they belong to a room and a moment, so every set covering that moment shows them; clicking one seeks everyone in Watch Party. Kinds other than notes, targets other than a moment, and tags are planned.

## Drawing (planned)

Part of the app. Drag on a paused frame to draw: pen, arrow and a fading laser, X clears, one colour per person. In Watch Party live strokes show to everyone at once; finished strokes are kept as events tied to the person, the moment and the view.

## Plugins (planned)

- Analysis only: a plugin reads recordings (picture and sound) and set information and writes events. It has no UI; the app shows every plugin's events the same way.
- Self-contained: its own program, depending on no other plugin and turned on or off on its own. The app knows nothing about its internals.
- Chosen per room with their settings, so everyone gets results from the same plugin versions and settings. Each person only chooses which results to show.
- Members share the work: a job is one plugin version and settings on one file, and any member that holds the file and is idle claims it in its log, runs it and shares the results there. Each member takes its own recording first. A claim that is not finished in time can be taken by another member; a job done twice gives the same result and is shown once.
- Jobs run per set file after recording by default, pause while recording, resume, and skip inputs already done by the same version. A light plugin may be allowed to run while recording.
- Official plugins only, kept in this repository and bundled.

## Transcription (planned)

A plugin that turns voice into range events with Whisper on whichever member runs the job, with a choice of models. Shown as subtitles on the video (switchable) and as a conversation list that scrolls with playback and seeks on click.

## Export

File > Export set writes one video on this PC, like a screen recording of the Player: the set's players on the auto grid with names, lined up as in playback; track 1 is the mix and one track per player follows. Players whose files are not here yet are left out.

## Deleting and cleanup

- Deleting a set removes it from this PC only and is remembered, so it is not fetched again; others keep theirs.
- Sets older than the kept days are deleted the same way (empty keeps them forever).
- When free disk space runs low, the oldest sets without notes go first, then those with notes, never one being recorded; the app says which. When the disk is nearly full, recording stops.
- The app removes old live segments, cancelled exports and old logs by itself. It never touches recordings, notes or the key without being asked.

## Planned

- Language: English and Japanese.
- Tab shortcuts: Ctrl+1, Ctrl+2 and Ctrl+Tab.
- Renaming a room.
- "App" wording in the UI for capture sources and app sound.
- Per-member receive volume and mute.
- Events beyond notes: targets, tags, filtering and search.
- Drawing.
- Plugins and shared jobs; first an Apex plugin for match boundaries, jumping between events and chapters.
- Transcription.
- Cleanup of leftovers from rooms left and sets deleted, and of old reports.
- Sync state as a colour in the top bar, with per-member connection, live delay and progress in the room menu.
