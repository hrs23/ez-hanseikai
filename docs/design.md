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
- A room is meant to be kept and reused, so its history builds up; a new room starts with its own empty history.
- Joining needs approval: someone using a code asks to join, every member online sees "<name> wants to join: Approve / Decline", and nothing of the room is sent until a member approves. The request ends with the invite.
- Any member can remove a member for the whole room: the removal is a record in the remover's log, so it reaches everyone; everyone stops sending to them from then on and refuses their connections, and everyone sees who removed whom. What they already received stays with them. A later approval lets them back in.
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
- One log per person per room, `rooms/<room>/logs/<member-id>.bin`. Members receive only that room's logs, and everyone keeps every member's log of the room, so the room survives anyone leaving. A person whose own log of a room is empty (a new PC with an imported key, or a deleted data folder) first gets it back from a member, then writes; until then new records wait in `pending.bin`.
- Logs in the old single-file format are split by room on first start, the same way on every PC; a record without a room goes with its set or note, and the old log is kept as `legacy/log.bin`, which is never sent.
- A set is a folder, `rooms/<room>/sets/<local start time>/`, with `<member-id>.mp4` for every member and `set.json`, which keeps the recorder's tile order and layout as the set's default.
- Log record kinds include `rec`, `rec_end`, `file`, `file_bad`, `event`, `event_edit`, `event_del`, `event_restore`, `remove`, `admit`, `offset` and `layout`; old `note` records are still read. Unknown kinds are kept and ignored. Edits and deletions are new records.
- The app works from an empty data folder, and recreates its files if the folder is deleted while it runs; without `key.bin` it makes a new identity at the next start and says so.

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
- Watch Party: everyone who turns it on sees the same thing, with no leader. One shared state holds the set, playback (playing, position as wall time, speed), view, layout and tile order; it is ordered by a Lamport counter, then time, then sender, the newest wins, and it is repeated every few seconds, so anyone who joins catches up. Each message also carries how many records of each member's log the sender holds and how much of the set it has. Those counts only grow; everyone shows only events below them, fetches what it lacks at once, and shows "Catching up with Ren" or "Ren receiving 72%" to all until it has it. Choosing a set switches everyone; a member without the files fetches that set first and shows its tiles as receiving where it has no picture yet. The screen says who did what, and the last seek can be undone. Volume stays personal; outside Watch Party the layout is each viewer's own.
- Volume is personal. Each person can set app sound and voice volume and mute for each member for themselves (planned; one volume now).

## Events

Notes, markers and plugin results are one kind of record, an event: who made it (the person whose log holds it, or a plugin name and version), its kind, an optional target (a moment, a range, a file or the whole set) and its content (text or numbers), with tags (people write `#tag` in a note; plugins tag their own). The filter field above the notes searches text and names, and `#tag` matches a tag. Only people's events can be edited or deleted. A moment belongs to the room, so every set covering it shows the event; clicking one seeks everyone in Watch Party. The app makes notes; other kinds come from plugins.

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
- The app removes by itself old live segments, live segments of rooms not open, partial exports, what is left of rooms left and sets deleted, and reports and old logs older than a week. It never touches recordings, events or the key without being asked.

## Planned

- Language: English and Japanese.
- Tab shortcuts: Ctrl+1, Ctrl+2 and Ctrl+Tab.
- Renaming a room.
- "App" wording in the UI for capture sources and app sound.
- Per-member receive volume and mute.
- Drawing.
- Plugins and shared jobs; first an Apex plugin for match boundaries, jumping between events and chapters.
- Transcription.
- Sync state as a colour in the top bar, with per-member connection, live delay and progress in the room menu.
- From the previous app (Apex Hanseikai), in the Player:
  - Notes overlaid on the video for a few seconds from their time, and "<name> is typing".
  - A quick note box: C pauses and opens a small box over the video, with an option for a note without a time; Enter adds and resumes.
  - Seekbar thumbnails on hover and the received part of each file.
  - Fill the window (T) besides full screen; click toggles play; double-tap skips on touch screens.
  - 3x speed, Ctrl+Left/Right and buttons for one minute, Shift+P/N for the previous and next set, A/D for the previous and next event of the chosen kind.
  - In Watch Party: viewer count, typing indicator and shared speed.
  - Copy a link to this moment and view, which opens the app there.
- A summary board per room: one card for the team and one per member, short Markdown, with links to moments; and a page per tag.
- Keeping only the parts of old sets around notes and events instead of deleting them whole.
- A storage view: free space and the space each set uses.
- A jobs view: plugin jobs with progress, retry and cancel.
- Thin connections: send in order of importance (logs and notes, sound, live picture, then history); members who already have a file pass it on, so a slow uploader sends each piece once; history slows down by itself when round trips grow; the room menu shows each member's speed and time left. Optional later: receiving only the sets one opens.
- Sending quality: Light / Standard / High presets, or choose the resolution (720p, 1080p, as captured) and frame rate (30, 60) yourself; the bit rate follows from them and the codec, and the room's total bandwidth is shown next to it.
- Receiving quality: each member can ask for a light live picture from a member; that member then also encodes a smaller live copy while anyone asks for it. Saved sets keep the sender's quality.
- Health check: Room > Check connections tests every pair of members (direct or relay, round trip, upload and download speed with a short transfer) and each member's capture, encoder, microphone and free space, and shows everyone the same table, with what to do for each problem.
