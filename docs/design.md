# Design

What the app does and how, as built. What is decided but not built is under [Planned](#planned).

## Shape

- A Windows app, the same for everyone. Tauri: a Rust core that captures, encodes and syncs; a web UI. Closing the window quits the app, asking first only while a session records, files arrive or an export runs; minimizing keeps it in the taskbar. One app runs per data folder: a second start brings the running window to the front, and an app on the same folder from another build says so and exits (a lock file `app.lock` in the data folder; one left behind by a crash makes the next start say the app restarted after a problem).
- One window: a left sidebar that is the room, and a main area for the page chosen there. No tabs and no top bar. A native menu bar (File, Room, Session, View, Help) mirrors the commands. Each action and piece of information has one place, with no explanatory text; status sits next to what it affects. Logs stay in the diagnostics window.
- The sidebar, top to bottom: the room switcher; Room (the room's page, where the app opens); Now, the room's present, with Start session / End session; Sessions by day; Members with join requests and the open invite; and the own row (name, capture source, microphone mute), which opens Capture. Room-wide things are above, personal ones at the bottom. Ctrl+1 to Ctrl+3 open Room, Now and Capture; Ctrl+B hides the sidebar.
- Shared and personal stay apart: the room part of the sidebar and the player's outlined region are shared; the own row, Capture and the player header are personal. Destructive actions say whether they affect only this PC or everyone. Confirmations are in-app dialogs; file pickers and the few native message boxes come from the Windows dialogs.
- Everyday review features live in the app: recording, sharing, rooms, playback, notes, drawing, Watch Party, export and showing events. Game-specific analysis lives in plugins. The UI says "app" for capture sources and their sound; game terms such as matches come from plugins as tags and chapters.
- Nothing goes to a cloud. Everyone is equal: no host and no leader for recording or playback.
- English and Japanese, following the Windows display language until one is chosen in the settings. The UI looks up each English string in one dictionary (`ui/i18n.js`), so fixed messages from the core are translated the same way; the core sends codes for notices that carry values. The native menu, message boxes and notifications have their own small table.
- One icon set (Lucide, inlined in `ui/icons.js`) and one set of design tokens in `ui/style.css`.
- Each member has a colour, the same on every PC: members (and removed members) sorted by ID take the palette in order. It marks their avatar, notes and drawings.

## Identity

One key per person, made on first start; its public half is the person's ID everywhere. The name can change; the ID cannot. Moving to a new PC means exporting and importing the key (File menu).

## Rooms

- Without a room the window shows the start screen: the name (only until one is set), Create room, Join with a code, and the rooms joined before. While a join waits for approval it shows that, with Cancel.
- The room switcher switches, creates and joins rooms. The Room page shows the name, how many are online, a menu (rename, copy an invite code, leave), the board and the room's plugins.
- Members in the sidebar: avatar (a red dot while recording), name and round trip (the warning colour on a relay), or Offline. A member's popover shows the connection and their check results and removes them for everyone. Check connections and Copy invite code are icons on the Members header; an open invite is a row there with copy and cancel.
- Invite codes work once, only while the inviter's app runs, and can be cancelled. Creating a room copies an invite code. Any member can invite. Joined rooms stay in the list; one is open at a time.
- Any member can rename the room: a `room_name` record in their log; the newest (by each log's clock offset) wins everywhere.
- A room is meant to be kept and reused, so its history builds up; a new room starts with its own empty history.
- Joining needs approval: someone using a code asks to join, every member online sees a row "<name> wants to join" with Approve and Decline in the Members list (and a Windows notification while the window is minimized), and nothing of the room is sent until a member approves. The request ends with the invite.
- Any member can remove a member for the whole room: the removal is a record in the remover's log, so it reaches everyone; everyone stops sending to them from then on and refuses their connections, and everyone sees who removed whom. What they already received stays with them. A later approval lets them back in.
- Leaving a room removes it from this PC only.

## Live and recording

- Capture is a personal page: the source (a window or a whole display, grouped, displays with their size), the own preview with Live and REC badges, the resolution, frame rate, bit rate and codec in use, and chips for screen, app sound, microphone and encoder; then sound (the app whose sound is recorded, the microphone, meters, volumes and mutes), quality, Send to room and the upload limit. Every start begins with no source and the microphone muted, so nothing is sent until the player chooses what to capture; the choice is not kept for the next start. A capture problem shows on the preview and on the own row. A shared window that closes stops the capture and shows as lost; the app never moves to another window of the same program (only the same window may change its title).
- The microphone is its own track, muted at every start, and can be unmuted and muted again (also from the own row). The volumes of app sound and microphone are kept on this PC. App sound comes from per-process capture. Each source has a dB meter with peak and clipping and a volume.
- Inside a room everyone's picture and sound stream all the time; Now shows them live, a few seconds behind. Send to room off sends nothing; the upload limit caps the upload.
- The recording unit is a session, one per room: anyone's Start session (Now row, Session menu or Ctrl+R) starts it for every member and anyone's End session ends it; it needs a capture source. Every member sees the same list. Nothing is kept while nobody records. A member who joins mid-session records from then on; an absent member is an empty tile. While a session records, the Now row shows the timer and who started it, and opens the player at the live edge. (Records and code call a session a set.)
- Mark this moment (Session menu or a global hotkey, Ctrl+Alt+M by default and changeable in Settings) logs a `mark` event at the current time of the recording session. The hotkey is registered with Windows only while a session records and never touches the game.
- The capture is scaled down to the quality's width and converted to NV12 (BT.709) on the GPU before it reaches ffmpeg, or on the CPU when the GPU cannot. One ffmpeg process per person encodes it once into two fragmented MP4 series that it sends to the app over localhost: `v` (picture and app sound) and `m` (voice). Each fragment is one second. The app cuts them into segments of 4 fragments, and also at once on Record and Stop, so a set starts and ends on a fragment. Four seconds keeps a live joiner close to now and keeps the file count low (about 900 per hour per series). Live and recording are the same segments: a recording is the list of segments logged for the set.
- A capture is one encoder run; a restart (new size, codec or quality, or a crash) starts a new capture, so a set can have several parts per member. A new size or quality waits until no set is recording. An encoder that exits, or gives no output for 10 s while frames keep coming or a frame write does not return, is stopped and restarted into the same set; after three failures in a row without a minute of output it is skipped for ten minutes for the next working encoder (another backend of the same codec first), down to libx264. The old ffmpeg is stopped before the next one starts. Which encoders work is probed once, each probe with a time limit, and kept until ffmpeg or the GPU driver changes. Until the new encoder gives output, the preview and the own row show the problem and the own tile does not show recording. Screen capture never waits on ffmpeg (a feeder thread per capture writes only the newest frame), and sound writes to it time out.
- Quality presets, each with its width at 30 fps: Light 960, Standard 1280 (the default), High 1920. The height follows the source's shape and nothing is scaled up. The bit rate is 0.2 bits per pixel per frame of the preset's 16:9 size for H.264, times 0.7 for HEVC and 0.55 for AV1 (Standard: 5.5 Mbit/s H.264, 3.0 Mbit/s AV1). Capture shows the resolution, frame rate, bit rate and codec in use.
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
- One connection per pair of members, used both ways: the member with the lower ID dials, and someone joining dials the member whose invite they use. A new connection from a peer that still has a working one (it answers a ping within 3 s) is refused, so two connections never close each other. A failed dial, or a connection that ends within 10 s, doubles the wait before the next dial, from 2 s up to 30 s, with a quarter of random spread so peers do not dial in step. While a join waits for approval, the joiner asks again every log sync on the same connection.
- On connecting, members pull every member's log from any peer that has it, so late or offline members learn every set.
- Media moves with one request, follow: from a segment and byte, the sender streams segments in order, waits on the open one and goes on into new ones. Live follows each member's current segment; missing segments of sets come the same way from the owner or any member with verified copies, resuming where they stopped.
- Order of importance: logs, voice, live picture, then history (voice before picture). The speed limit caps the upload; history always keeps a quarter of it, so live streams cannot stop it.
- The player and Now read media through the app's `ezh://` scheme, which joins the init and segments into one growing file with byte ranges.

## Verification

Each recorder logs the size and SHA-256 of every init and set segment. Every member hashes each copy once complete; a copy that does not match is logged as `file_bad`, deleted and fetched again from a good copy. Members tell each other, per set, member and series, how many segments they hold verified in order, so the set list shows whether everyone has everything or what is missing.

## Time

Peers measure clock offsets continuously and log them. Playback lines up everyone in a set from the recording start times and the offsets, with a manual nudge.

## Player and Watch Party

- The player opens from Now or a session row. Its header is personal (title, Watch Party switch, subtitles, volume, events panel, full screen); below it, the shared region holds the views, layout, drawing tools, tiles, chapters, event marks, seekbar and transport, outlined in the accent colour while Watch Party is on, with the other viewers' avatars on its border. The events panel on the right lists events (or the conversation).
- Each person plays their own copy. Views: all players, or one player's picture and sound (keys 0 and 1 to 6).
- Layouts: a two-tier picker chooses the number of places (1 to 6, never fewer than the players present; the rest stay free) and then auto grid, one large with the others beside or below, or a row. Dragging a name label swaps two players. The recorder's layout is the set's default; each viewer can rearrange.
- Click toggles play, double-click shows one player, F is full screen. Shortcuts (View > Keyboard shortcuts): Space/K, arrows and J/L to skip (Shift and Ctrl+arrows for longer), < > for speed up to 3x, Shift+P/N for the previous and next session, Q/E for notes, A/D for a minute before the previous or next match end (other events and chapters when there are no matches), C for a quick note, X to clear drawings, Ctrl+Z to undo the last seek or deletion.
- Watch Party: everyone who turns it on sees the same thing, with no leader. One shared state holds the set, playback (playing, position as wall time, speed), view, layout and tile order; it is ordered by a Lamport counter, then time, then sender, the newest wins, and it is repeated every few seconds, so anyone who joins catches up. Turning it on joins what the others are watching and never moves them: a state from turning on has counter 0 and the negative turn-on time, so any action and any earlier turn-on wins over it. Each message also carries how many records of each member's log the sender holds and how much of the set it has. Those counts only grow; everyone shows only events below them, fetches what it lacks at once, and shows "Catching up with Ren" or "Ren receiving 72%" to all until it has it, or "Ren incomplete at 72%" when no member online has more. Choosing a set switches everyone; a member without the files fetches that set first and shows its tiles as receiving where it has no picture yet. The screen says who did what, and the last seek can be undone. Volume stays personal; outside Watch Party the layout is each viewer's own.
- Each tile plays its picture and app sound and, as a separate audio stream on the same clock, its voice. Everyone's voice is heard in every view; app sound comes from the chosen player.
- Volume is personal: one volume and mute for the player, and, from a player's name label, app sound and voice volume and mute for that member, kept on this PC.
- Hovering the seekbar shows the frame of the shown player at that time (a hidden copy of the player seeks there), and the chapter.
- Sync status sits on what it concerns: a session row shows this PC's receiving (or where it stopped), the running analysis and an export; a tile shows how much of that member's file is here; a member row shows when this PC is catching up with that member's log.

## Events

Notes, markers, drawings, board cards and plugin results are one kind of record, an event: who made it (the person whose log holds it, or a plugin name and version), its kind, an optional target (a moment, a range, a file or the whole set) and its content (text or numbers), with tags (people write `#tag` in a note; plugins tag their own). Only people's events can be edited or deleted, and a deletion can be undone. A moment belongs to the room, so every set covering it shows the event; clicking one seeks everyone in Watch Party. The app makes notes, drawings and board cards; other kinds come from plugins.

The Player shows every event by its shape, never by plugin:

- A moment: a mark on the seekbar, a row in the event list, and, with text, a line over the video for 8 seconds.
- A range on one member's file with text (such as speech): a subtitle on that member's tile (CC switches it) and a line in the Conversation list, which follows playback and seeks on click.
- Any other range: a chapter above the seekbar and a row in the list.
- Moments a plugin tags `match` are match ends: the chapters between them are "Match 1, 2, …" (the open one only while recording), and the current match shows beside the clock.

The filter above the list searches text, names and kinds, and `#tag` (or clicking a tag) lists that tag's events, in this session or, with its scope, in all sessions of the room. A quick note (C or the note button) pauses, opens a box over the video at that time (Tab: without a time, or completes a tag), and Enter adds it and resumes; while it is open others in Watch Party see "<name> is typing". Right-clicking the video copies a link to the moment and view (`ezh://moment/<set>/<seconds>/<member>`); links in notes and on the board open it in the app.

The board (Room page) has one card for the team and one per member: short Markdown (headings, lists, bold, `==highlight==`, links to moments and tags), each card the newest `board` event for it.

## Drawing

Part of the app. Drag on a paused frame to draw: pen, arrow and a laser that fades (the laser works while playing too), X clears, one colour per person. Points are kept relative to the drawn player's picture, so strokes stay in place whatever the layout. In Watch Party strokes go to everyone as they are drawn; finished pen and arrow strokes are kept as `drawing` events (player, tool, points, moment) and show whenever playback is paused at that moment. Clearing hides the strokes at that moment for everyone until playback moves on.

## Plugins

- Analysis only: a plugin reads recordings (picture and sound) and set information and writes events, plus an optional tag dictionary (tag to display name). It has no UI; the app shows every plugin's events the same way. It never touches the game.
- Self-contained: its own program, depending on no other plugin and turned on or off on its own. The app knows nothing about its internals. Each plugin runs as a separate process of the app (`ez-hanseikai.exe --plugin`, idle priority; every child process of the app ends with it): the job comes as JSON on stdin (plugin, settings, set and file information, duration, picture, app sound and voice as file parts plus a stream, the ffmpeg path, its own folder `plugins/<id>` and a scratch folder), and it answers with JSON lines on stdout, `{"progress": 0.4}` and finally `{"output": {"events": [...], "tags": {...}}}` or `{"error": "..."}`. Event times are seconds from the start of the file (`at_s`, or `from_s` and `to_s`); the app turns them into wall times.
- A plugin declares an id, a name, a version, whether it may run while recording, its settings as lists of choices (the first is the default), and the files it needs for given settings (URL, SHA-256, size). Those are downloaded into `plugins/<id>` only when a member asks; until then that PC runs none of its jobs.
- Chosen per room with their settings, so everyone gets results from the same plugin versions and settings: a `plugin` record holds the plugin, on or off, the version of whoever set it, and the settings; the newest wins. Only members with that version run its jobs. Each person only chooses which results to show.
- A plugin whose result is about the whole set rather than one member (it declares so) runs once per set, on the capture with the most picture segments (the lowest name on a tie), which every member derives the same from the logs; only that job's results are shown. Other plugins run on every file.
- Members share the work: a job is one plugin version and settings on one file (its key is `<plugin>@<version>/<settings hash>/<set>/<owner>/<capture>`; a member's part of a set from one capture is read as its `init.mp4` and segments in order). Any member that holds a verified copy and is idle claims it with a `job` record in its log (`state` `claim`, then `done`, `fail` or `stop`, each with `job`, `run`, `plugin`, `version` and `file`; `retry` clears its own failure; a claim waits two log syncs and, if a connected member claimed the job earlier or another member already finished it, gives way with `yield`), runs it and writes the events and then `done` there. Its own unfinished claim goes first, then its own recordings, then the newest sets. A claim by a member who is not connected can be taken by another member. Of several `done` runs of a job only one is shown (the lowest member ID), and events of a run without `done` are never shown, so an interrupted run leaves nothing behind and reruns from scratch.
- Only results of jobs that match the room's current plugin choice are shown; changing a setting or the version starts new jobs.
- Jobs run one at a time per PC on finished files, stop when recording starts (unless the plugin may run while recording) and rerun later, and skip files already done. A run stopped on purpose (recording, a changed choice, exit or an update) logs `stop`. A job this PC claimed twice without `done`, `fail` or `stop` is logged as failed instead of being claimed a third time: one interruption can have another cause, two on the same job point to the job.
- Settings has "Run analysis on this PC" (on by default), so a weak PC can leave the jobs to the others.
- A plugin's output lines are read up to 16 MB each, far more than any result; a longer line fails the job.
- Official plugins only, kept in this repository and bundled.
- The Room page has a card per plugin: on or off and its settings for the room, and whether it is ready on this PC, with a download. A running job's progress and a failure show on its session row, whose menu retries; the full job list is in the diagnostics window.

### Apex matches

Finds match ends from key frames, scaled down to 64 by 36: the result screen, a gray picture with a red banner across it, marks the end where it first shows; ends closer than 150 seconds count once. Moment events of kind `match_end`, tagged `match`. Match ends are the same for the whole squad, so it runs once per set.

## Transcription

A plugin that turns each member's voice into range events of kind `speech` with whisper.cpp (its Windows CLI, on half of the cores) on whichever member runs the job. Settings: model (small, base, medium, large-v3-turbo) and language (auto or a fixed one). The tool and the chosen model are downloaded on request. Shown as subtitles on the video (switchable) and as a conversation list that scrolls with playback and seeks on click. whisper runs without carrying text from one window to the next (`-mc 0`), which stops it repeating a line forever. Before it, ffmpeg finds silence (below -45 dB for at least a second) and levels the speech (`speechnorm`); a line with less than a quarter second of sound under it, or one that repeats one of the three lines before it, is dropped.

## Health check

Check connections (the Members header) runs the local check (Windows build, capture, app sound, encoder, relay and UDP) on every connected member at once and shows each member's result in their popover, with a mark on their row when something failed.

## Storage

File > Storage shows the data folder's size, the free space, how many days sessions are kept and the space each session uses here, each with the session menu.

## Export

Export video… (Session menu or a session row) writes one video on this PC, like a screen recording of the player: the set's players on the auto grid with names, lined up as in playback; track 1 is the mix and one track per player (app sound and voice) follows. The app streams each part's segments to ffmpeg over localhost. Players with nothing here yet are left out. Each tile is as wide as the widest player's picture (the whole video at most 2560 by 1440), and the bit rate is the sum of the players' bit rates, turned into H.264 terms, but no more than the preset rule gives for the video's size: the export cannot hold more picture than its sources. The video is written in the data folder first and then moved into place, so an existing file is replaced only by a finished one.

## Deleting and cleanup

- Delete from this PC… removes a session from this PC only and is remembered, so it is not fetched again; others keep theirs.
- Sets older than the kept days are deleted the same way (empty keeps them forever).
- When free disk space runs low, the oldest sets without notes go first, then those with notes, never one being recorded; the app says which. When the disk is nearly full, recording stops.
- Deleting a set removes its segments. A segment that is in no set goes once it is not among the newest few of a live capture; a capture folder goes when nothing in it is kept. The app also removes partial exports, what is left of rooms left, and reports and old logs older than a week. It never touches recordings, events or the key without being asked.

## Diagnostics

Help > Diagnostics or Ctrl+Shift+I opens a separate window, like browser developer tools, with tabs: Connections (each peer's paths, round trip, speed, clock offset, version and connection events; addresses only when shown), Network (every segment and log transfer with its result, the streams by priority, the pacer and the message counts), Recording (capture, frames, the encoder chosen and why, segments as they are cut, ffmpeg output), Data (each room's logs, what each member holds of a set, verification and the Watch Party state), Jobs (job records in the logs), Log, Report and App (check, data folder, reset). The window polls the core about once a second while it is visible.

The text log `log.txt` has the local date and time on each line and logs joins, sessions with their segment counts, captures, exports, updates, plugin jobs, connections and path changes, and errors; it rolls over at 10 MB into `log.old.txt` and `log.older.txt`. A crash also writes `crash.txt` at once, which the report includes.

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
