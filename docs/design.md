# Design

What the app does and how, as built. What is decided but not built is under [Planned](#planned).

## Shape

- A Windows app, the same for everyone. Tauri: a Rust core that captures, encodes and syncs, and keeps running in the tray with the window closed; a web UI.
- Three tabs, Room, Record and Player (Ctrl+1 to Ctrl+3, Ctrl+Tab), a native menu bar and a tray icon. Each action and piece of information has one place, with no explanatory text. Logs and diagnostics stay in the debug window.
- Everyday review features live in the app: recording, sharing, rooms, playback, notes, drawing, Watch Party, export and showing events. Game-specific analysis lives in plugins. The UI says "app" for capture sources and their sound; game terms such as matches come from plugins as tags and chapters.
- Nothing goes to a cloud. Everyone is equal: no host and no leader for recording or playback.
- English and Japanese, following the Windows display language until one is chosen in the settings. The UI looks up each English string in one dictionary (`ui/i18n.js`), so fixed messages from the core are translated the same way; the native menu has its own small table.
- Each member has a colour, the same on every PC: members (and removed members) sorted by ID take the palette in order. It marks their avatar, notes and drawings.

## Identity

One key per person, made on first start; its public half is the person's ID everywhere. The name can change; the ID cannot. Moving to a new PC means exporting and importing the key (File menu).

## Rooms

- Without a room, both tabs show the start screen: the name (only until one is set), Create room, Join with a code, and the rooms joined before. While a join waits for approval it shows that, with Cancel.
- In a room the top bar shows the room name with everyone's avatar (dimmed when not connected, a red dot while recording); it switches, creates and joins rooms.
- The Room tab is the room's home and where the app opens: the name and how many are online, Copy invite code and a menu (cancel the invite, rename, check connections, plugins, leave); a card per member with their state and connection (direct or relay and round trip; relay is marked as a thin connection; right-click removes); the board; the newest sets and notes, which open the Player there.
- Invite codes work once, only while the inviter's app runs, and can be cancelled. Creating a room copies an invite code. Any member can invite. Joined rooms stay in the list; one is open at a time.
- Any member can rename the room: a `room_name` record in their log; the newest (by each log's clock offset) wins everywhere.
- A room is meant to be kept and reused, so its history builds up; a new room starts with its own empty history.
- Joining needs approval: someone using a code asks to join, every member online sees "<name> wants to join: Approve / Decline", and nothing of the room is sent until a member approves. The request ends with the invite.
- Any member can remove a member for the whole room: the removal is a record in the remover's log, so it reaches everyone; everyone stops sending to them from then on and refuses their connections, and everyone sees who removed whom. What they already received stays with them. A later approval lets them back in.
- Leaving a room removes it from this PC only.

## Live and recording

- The source is a window or a whole display, chosen per person; it starts as none, and nothing is captured or sent until one is chosen. Others then see a placeholder tile with the name. While a whole display is shared, the top bar says so.
- The microphone is on by default, as its own track, and can be muted. App sound comes from per-process capture. Each source has a dB meter with peak and clipping and a volume.
- Inside a room everyone's picture and sound stream all the time and show live in the Record tab, a few seconds behind. Sharing off sends nothing; one speed limit caps the upload.
- One recording per room: anyone's Record starts it for every member and anyone's Stop ends it. That span is a set holding everyone's file, and every member sees the same list. Nothing is kept while nobody records. A member who joins mid-set records from then on; an absent member is an empty tile.
- One ffmpeg process per person encodes the capture once (scaled down to the quality's width) into two fragmented MP4 series that it sends to the app over localhost: `v` (picture and app sound) and `m` (voice). Each fragment is one second. The app cuts them into segments of 4 fragments, and also at once on Record and Stop, so a set starts and ends on a fragment. Four seconds keeps a live joiner close to now and keeps the file count low (about 900 per hour per series). Live and recording are the same segments: a recording is the list of segments logged for the set.
- A capture is one encoder run; a restart (new size, codec or quality, or a crash) starts a new capture, so a set can have several parts per member. A new size or quality waits until no set is recording. An encoder that exits, or gives no output for 10 s while frames keep coming or a frame write does not return, is stopped and restarted into the same set; after three failures in a row without a minute of output it is skipped for the next working encoder, down to libx264. Until the new encoder gives output, the top bar shows the problem and neither the own tile nor the tray shows recording. Screen capture never waits on ffmpeg (a feeder thread per capture writes only the newest frame), and sound writes to it time out.
- Quality presets, each with its width at 30 fps: Light 960, Standard 1280 (the default), High 1920. The height follows the source's shape and nothing is scaled up. The bit rate is 0.2 bits per pixel per frame of the preset's 16:9 size for H.264, times 0.7 for HEVC and 0.55 for AV1 (Standard: 5.5 Mbit/s H.264, 3.0 Mbit/s AV1). The Record tab shows the resolution, frame rate, bit rate and codec in use.
- Meters show the level after the volume, in dB from -60 to 0 like OBS: green, yellow from -20 dB, red from -9 dB, the highest peak held for 1.5 s, and a clip mark for 3 s after a full-scale sample.
- Codec: AV1, then HEVC, then H.264, the best one that every member can play. A keyframe every second, no B-frames. There is no separate full-quality copy.

## Data

- Each person writes their own append-only files: their recordings and their log. The logs are the source of truth; a torn last record is dropped on start.
- One log per person per room, `rooms/<room>/logs/<member-id>.bin`. Members receive only that room's logs, and everyone keeps every member's log of the room, so the room survives anyone leaving. A person whose own log of a room is empty (a new PC with an imported key, or a deleted data folder) first gets it back from a member, then writes; until every member has said how much of it they hold (or one connection attempt has passed) and none holds more, new records wait in `pending.bin`.
- Media lives in `rooms/<room>/<member-id>/<capture start ms>/{v,m}/`: `init.mp4` and `000001.m4s`, `000002.m4s`, and so on, written as `.part` and renamed when complete. `cap.json` keeps the capture's start and settings, and `open.json` the segment being written, for crash recovery.
- The owner logs `cap` (capture start, settings, size and SHA-256 of each init) and `seg` (capture, series, number, set, start, duration, size and SHA-256) for segments in a set only; live-only segments are never logged. A member's part of a set is its `seg` records for that set. The set's default layout is the last `layout` record for it.
- After a crash, the open segment is cut back to its last whole fragment, finished and logged, and an open set gets its `rec_end`; if the set is still recording elsewhere, the member joins it again with a new `rec`. A Stop marks its `rec_end` (`stop`), so a member that missed the message ends the set when it finds that record in any log.
- Log record kinds include `rec`, `rec_end`, `cap`, `seg`, `file_bad`, `event`, `event_edit`, `event_del`, `event_restore`, `remove`, `admit`, `room_name`, `offset`, `layout`, `plugin` and `job`. Unknown kinds are kept and ignored. Edits and deletions are new records.
- The app works from an empty data folder, and recreates its files if the folder is deleted while it runs; without `key.bin` it makes a new identity at the next start and says so.

## Sharing

- Peers connect directly with iroh, through a relay when they must. Connections are encrypted and accepted only from room members. Address lookup publishes only the relay URL.
- On connecting, members pull every member's log from any peer that has it, so late or offline members learn every set.
- Media moves with one request, follow: from a segment and byte, the sender streams segments in order, waits on the open one and goes on into new ones. Live follows each member's current segment; missing segments of sets come the same way from the owner or any member with verified copies, resuming where they stopped.
- Order of importance: logs, voice, live picture, then history (voice before picture). The speed limit caps the upload; history always keeps a quarter of it, so live streams cannot stop it.
- The Player and the Record tab read media through the app's `ezh://` scheme, which joins the init and segments into one growing file with byte ranges.

## Verification

Each recorder logs the size and SHA-256 of every init and set segment. Every member hashes each copy once complete; a copy that does not match is logged as `file_bad`, deleted and fetched again from a good copy. Members tell each other, per set, member and series, how many segments they hold verified in order, so the set list shows whether everyone has everything or what is missing.

## Time

Peers measure clock offsets continuously and log them. Playback lines up everyone in a set from the recording start times and the offsets, with a manual nudge.

## Player and Watch Party

- Each person plays their own copy. Views: all players, or one player's picture and sound (keys 0 and 1 to 6).
- Layouts, on both tabs: a two-tier picker chooses the number of places (1 to 6, never fewer than the players present; the rest stay free) and then auto grid, one large with the others beside or below, or a row. Dragging a name label swaps two players. The recorder's layout is the set's default; each viewer can rearrange.
- Click toggles play, double-click shows one player, T fills the window, F is full screen. Shortcuts (View > Keyboard shortcuts): Space/K, arrows and J/L to skip (Shift and Ctrl+arrows for longer), < > for speed up to 3x, Shift+P/N for the previous and next set, Q/E for notes, A/D for other events and chapters, C for a quick note, X to clear drawings, U to undo the last seek.
- Watch Party: everyone who turns it on sees the same thing, with no leader. One shared state holds the set, playback (playing, position as wall time, speed), view, layout and tile order; it is ordered by a Lamport counter, then time, then sender, the newest wins, and it is repeated every few seconds, so anyone who joins catches up. Each message also carries how many records of each member's log the sender holds and how much of the set it has. Those counts only grow; everyone shows only events below them, fetches what it lacks at once, and shows "Catching up with Ren" or "Ren receiving 72%" to all until it has it, or "Ren incomplete at 72%" when no member online has more. Choosing a set switches everyone; a member without the files fetches that set first and shows its tiles as receiving where it has no picture yet. The screen says who did what, and the last seek can be undone. Volume stays personal; outside Watch Party the layout is each viewer's own.
- Each tile plays its picture and app sound and, as a separate audio stream on the same clock, its voice. Everyone's voice is heard in every view; app sound comes from the chosen player.
- Volume is personal: one volume and mute for the player, and, from a player's name label, app sound and voice volume and mute for that member, kept on this PC.
- Hovering the seekbar shows the frame of the shown player at that time (a hidden copy of the player seeks there), and the chapter.
- Sync status has one place, the top bar: Watch Party's "Catching up with Ren", "Ren receiving 72%" and "Ren incomplete at 72%", and this PC's own receiving with speed and time left.

## Events

Notes, markers, drawings, board cards and plugin results are one kind of record, an event: who made it (the person whose log holds it, or a plugin name and version), its kind, an optional target (a moment, a range, a file or the whole set) and its content (text or numbers), with tags (people write `#tag` in a note; plugins tag their own). Only people's events can be edited or deleted, and a deletion can be undone. A moment belongs to the room, so every set covering it shows the event; clicking one seeks everyone in Watch Party. The app makes notes, drawings and board cards; other kinds come from plugins.

The Player shows every event by its shape, never by plugin:

- A moment: a mark on the seekbar, a row in the event list, and, with text, a line over the video for 8 seconds.
- A range on one member's file with text (such as speech): a subtitle on that member's tile (CC switches it) and a line in the Conversation list, which follows playback and seeks on click.
- Any other range: a chapter above the seekbar, a row in the list, and the current chapter's name beside the clock.

The filter above the list searches text, names and kinds in every set of the room, and `#tag` (or clicking a tag) lists that tag's events, so each tag has its own page. A quick note (C or the note button) pauses, opens a box over the video at that time (Tab: without a time, or completes a tag), and Enter adds it and resumes; while it is open others in Watch Party see "<name> is typing". Right-clicking the video copies a link to the moment and view (`ezh://moment/<set>/<seconds>/<member>`); links in notes and on the board open it in the app.

The board (Room tab) has one card for the team and one per member: short Markdown (headings, lists, bold, `==highlight==`, links to moments and tags), each card the newest `board` event for it.

## Drawing

Part of the app. Drag on a paused frame to draw: pen, arrow and a laser that fades (the laser works while playing too), X clears, one colour per person. Points are kept relative to the drawn player's picture, so strokes stay in place whatever the layout. In Watch Party strokes go to everyone as they are drawn; finished pen and arrow strokes are kept as `drawing` events (player, tool, points, moment) and show whenever playback is paused at that moment. Clearing hides the strokes at that moment for everyone until playback moves on.

## Plugins

- Analysis only: a plugin reads recordings (picture and sound) and set information and writes events, plus an optional tag dictionary (tag to display name). It has no UI; the app shows every plugin's events the same way. It never touches the game.
- Self-contained: its own program, depending on no other plugin and turned on or off on its own. The app knows nothing about its internals. Each plugin runs as a separate process of the app (`ez-hanseikai.exe --plugin`, below normal priority): the job comes as JSON on stdin (plugin, settings, set and file information, duration, picture, app sound and voice as file parts plus a stream, the ffmpeg path, its own folder `plugins/<id>` and a scratch folder), and it answers with JSON lines on stdout, `{"progress": 0.4}` and finally `{"output": {"events": [...], "tags": {...}}}` or `{"error": "..."}`. Event times are seconds from the start of the file (`at_s`, or `from_s` and `to_s`); the app turns them into wall times.
- A plugin declares an id, a name, a version, whether it may run while recording, its settings as lists of choices (the first is the default), and the files it needs for given settings (URL, SHA-256, size). Those are downloaded into `plugins/<id>` only when a member asks; until then that PC runs none of its jobs.
- Chosen per room with their settings, so everyone gets results from the same plugin versions and settings: a `plugin` record holds the plugin, on or off, the version of whoever set it, and the settings; the newest wins. Only members with that version run its jobs. Each person only chooses which results to show.
- Members share the work: a job is one plugin version and settings on one file (its key is `<plugin>@<version>/<settings hash>/<set>/<owner>/<capture>`; a member's part of a set from one capture is read as its `init.mp4` and segments in order). Any member that holds a verified copy and is idle claims it with a `job` record in its log (`state` `claim`, then `done` or `fail`, each with `job`, `run`, `plugin`, `version` and `file`; `retry` clears its own failure; a claim waits two log syncs and, if a connected member claimed the job earlier or another member already finished it, gives way with `yield`), runs it and writes the events and then `done` there. Its own unfinished claim goes first, then its own recordings, then the newest sets. A claim by a member who is not connected can be taken by another member. Of several `done` runs of a job only one is shown (the lowest member ID), and events of a run without `done` are never shown, so an interrupted run leaves nothing behind and reruns from scratch.
- Only results of jobs that match the room's current plugin choice are shown; changing a setting or the version starts new jobs.
- Jobs run one at a time per PC on finished files, stop when recording starts (unless the plugin may run while recording) and rerun later, and skip files already done.
- Official plugins only, kept in this repository and bundled.
- Plugins (Room tab menu) turns each plugin on or off with its settings, downloads what it needs on this PC, and lists the room's jobs (waiting, running with progress, done, failed with the error) with Cancel and Retry. The running job's progress shows in the top bar.

### Apex matches

Finds match ends from key frames, scaled down to 64 by 36: the result screen, a gray picture with a red banner across it, marks the end where it first shows; ends closer than 150 seconds count once. Moment events of kind `match_end`, tagged `match`.

## Transcription

A plugin that turns each member's voice into range events of kind `speech` with whisper.cpp (its Windows CLI) on whichever member runs the job. Settings: model (small, base, medium, large-v3-turbo) and language (auto or a fixed one). The tool and the chosen model are downloaded on request. Shown as subtitles on the video (switchable) and as a conversation list that scrolls with playback and seeks on click.

## Health check

Check connections (Room tab menu) runs the local check (Windows build, capture, app sound, encoder, relay and UDP) on every connected member at once and shows each member's result with their connection.

## Storage

File > Storage shows the data folder's size, the free space and the space each set uses here; a set's menu exports, opens or deletes it.

## Export

File > Export set writes one video on this PC, like a screen recording of the Player: the set's players on the auto grid with names, lined up as in playback; track 1 is the mix and one track per player (app sound and voice) follows. The app streams each part's segments to ffmpeg over localhost. Players with nothing here yet are left out.

## Deleting and cleanup

- Deleting a set removes it from this PC only and is remembered, so it is not fetched again; others keep theirs.
- Sets older than the kept days are deleted the same way (empty keeps them forever).
- When free disk space runs low, the oldest sets without notes go first, then those with notes, never one being recorded; the app says which. When the disk is nearly full, recording stops.
- Deleting a set removes its segments. A segment that is in no set goes once it is not among the newest few of a live capture; a capture folder goes when nothing in it is kept. The app also removes partial exports, what is left of rooms left, and reports and old logs older than a week. It never touches recordings, events or the key without being asked.

## Debug

Help > Debug or Ctrl+Shift+I opens a separate window, like browser developer tools, with tabs: Connections (each peer's paths, round trip, speed, clock offset, version and connection events; addresses only when shown), Network (every segment and log transfer with its result, the streams by priority, the pacer and the message counts), Recording (capture, frames, the encoder chosen and why, segments as they are cut, ffmpeg output), Data (each room's logs, what each member holds of a set, verification and the Watch Party state), Jobs (job records in the logs), Log, Report and App (check, data folder, reset). The window polls the core about once a second while it is visible.

Memory is bounded: the last 1500 transfers (about five minutes of a full room of six), 500 connection events, 120 cut segments (four minutes of both series), 200 ffmpeg lines; the Log tab reads at most the last 256 kB of `log.txt`.

Copy report and Save report collect the version, OS, GPU, check, connections, transfers, recent errors and the recent log, with addresses, IDs and names removed unless "Include addresses and name" is ticked. "Include everyone in the room" asks each connected member for theirs over the existing connection; each member answers with its own choice (addresses only while that member has the box ticked) and never with other members' addresses.

## Planned

- Double-tap to skip on touch screens.
- Choosing which plugin results to show, per person.
- Opening a moment link from outside the app (a registered `ezh:` link).
- Keeping only the parts of old sets around notes and events instead of deleting them whole.
- Thin connections: members who already have a segment pass it on, so a slow uploader sends each piece once; history slows down by itself when round trips grow; the room menu shows each member's speed and time left. Optional later: receiving only the sets one opens.
- Sending quality: besides the presets, choosing the resolution and frame rate (30, 60) yourself, with the room's total bandwidth shown next to it.
- Receiving quality: each member can ask for a light live picture from a member; that member then also encodes a smaller live copy while anyone asks for it. Saved sets keep the sender's quality.
- Health check: a transfer test between every pair of members, free space, and what to do for each problem.
