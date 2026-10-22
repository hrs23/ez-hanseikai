# Test cases

One list of every case the app is tested against, by area. Each line is a situation and its expected result, then where it is checked (unit and ui-test lines list their cases by file):

- `e2e`: a step of `scripts/e2e.mjs`, named as it prints it, with its `E2E_PART` (`live`, `recover` or `library`).
- `unit`: Rust tests, run by `cargo test` in `src-tauri`.
- `ui-test`: a test of `scripts/ui-test.mjs`, run by `node --test scripts/ui-test.mjs`.
- `manual`: only a person can check it.
- `todo`: known, with no test yet.

A test fails when an end-to-end step name is missing here. A new test or a new case adds its line.

## Rooms and invites

- start A and B, and nothing is written before A is in a room. `e2e` (`live`)
- B joins A's room with approval. `e2e` (`live`)
- C joins late through B's invite while B is cut off, approved on A once B is back. `e2e` (`live`)
- removing C for the whole room while a recording runs stops it fetching and leaves the others recording. `e2e` (`live`)
- B leaves the room from its menu and enters it again with its recordings and notes, A and B leave and enter again at once, a new room moves B out of it, and B deletes it from the list. `e2e` (`live`)
- leaving a room while one of its recordings is open shows no "Join a room first." toast. `manual`
- C removes the room from this PC while sharing and its folder does not come back. `e2e` (`live`)
- leaving or removing a room stops its capture before the folder goes, with no "could not finish" warning; a capture stop never waits for the kill timeout; a join leaves no empty pending folder; a peer that just restarted shows its live path as soon as data flows. `manual`
- B leaving ends only its part and rejoins, a crashed C does not keep the stopped set open, and everyone has it. `e2e` (`recover`)
- the room menu lists the room commands and a rename in its settings reaches everyone. `e2e` (`library`)
- each member row says one short state. `e2e` (`library`)
- B's chosen colour and built-in avatar and A's room icon reach everyone. `e2e` (`library`)
- A real two-PC test with friends: create a room, invite, join, approve, and see everyone's name, avatar, color and state. `manual`
- Picking a photo in Settings crops it to the middle and shows on the other PC. `manual`
- Pasting your own invite code is refused as not valid. `todo`
- A member dialing someone who left the room logs that the other app closed it as not a member. `manual`
- room: joining waits for an answer and ends the invite; the latest remove or admit wins; the latest room name wins on every clock; a rename made here is stamped one after the newest name known, even from a fast clock; invite tokens and room ids cannot leave the rooms folder; a peer can only name a room with an id this app makes; a member is present while connected or seen within a minute and joins again only after a minute unseen; colors are the same everywhere; colors stay when someone joins without a clash; a chosen colour replaces only its owner's colour and is the same everywhere; the newest profile in a member's own log gives their colour and avatar and a bad colour, an svg or an oversized image is skipped; a profile is written only when the look differs; writes go only to the open room once joined, never to a pending one; the newest room icon wins on every clock and an empty one clears it; settings drop a bad colour and avatar; a debrief links to the recording it names; ai del hides ai notes from every log but not human notes; board cards newest event wins and a card without id is the team card, and a card's size is clamped to the grid with a default when absent. `unit`
- Room settings: the picker offers the 16 room presets (flame, bolt, crown, skull, swords, shield, crosshair, rocket, wings, trophy, gem, comet, peak, mask, star, chevrons) and no animals, and picking one sets the icon to a PNG image of it; Remove returns to the id-picked default. `manual`
- a member row says one short state, also for a member reached through a relay; an offline avatar is dimmed whatever it shows; Settings shows a picked image as a pressed tile; an avatar shows the image or a built-in shape, and a member with none, an unknown, empty or odd avatar name shows a plain head-and-shoulders person (muted circle of their colour, lighter silhouette) and never the word undefined; the picker lists that person first, pressed while none is picked, and picking it clears the avatar, a room without an icon or with an unknown or odd icon name shows a room preset drawn in its colour on a dark tile with a thin border of that colour (the stored PNG looks the same), picked from its id, a room preset name wins over an old avatar name of the same word, an old avatar name still draws its animal in a room (the same on every PC), and a room with no id shows its initial. `ui-test`
- the avatar stack shows +n for the faces it cuts. `manual`

## Recording

- Record on A starts the set on B; the app then opens the Game plan, Live stays one click away in the sidebar, and starting a debrief changes nothing. `e2e` (`live`)
- leaving Live for another page and coming back shows the same text on the own live tile; a rebuilt tile always redraws its text. `e2e` (`live`), `ui-test`
- while the own live file exists and capture stalls, the live tile shows the capture problem as its small overlay line. `manual`
- a debrief row in the side panel offers Download when its audio is not on this PC, and Save once it is. `manual`
- while recording the start box says Stop recording with the time and the list gets the running recording. `e2e` (`live`)
- the set stops and everyone has it. `e2e` (`live`)
- A, B and C record a short set, C joining late and recording long enough that every tile has video at the far seek. `e2e` (`library`)
- Start pressed on A and B together makes one recording on every PC. `e2e` (`library`)
- the microphone, the mix and the hotkey are kept in the settings. `e2e` (`library`)
- A real two-PC recording of a real game with game sound and the microphone, lined up in playback. `manual`
- A long recording of an hour or more with three members runs without a stall, a growing delay or a lost part (soak test). `manual`
- The hardware encoders (NVENC, AMF, QSV) record on each GPU kind, and the fallback to the software encoder works. `manual`
- While recording with a game in focus, the global shortcut marks the moment and a notification says so; reset puts a changed shortcut back to its default. `manual`
- With the window behind another app, minimised or in the tray, a member coming online in the open room shows a notice with their avatar at the bottom right above the taskbar and plays the join chime, without taking focus or keys from the app in front; it fades after a few seconds, a click brings the window to the front, joins within a few seconds share one notice, nothing shows in the first seconds after start or for members already online, nothing shows while the window is in front or with the switch off. `manual`
- When another app holds the global shortcut at recording start, its Settings > Shortcuts row reads In use by another app until the keys are changed or reset. `manual`
- While recording with the window focused and no text field focused, M turns the microphone and V the screen picture off and on like the dock buttons (V asks before it stops sending), the buttons' tooltips show (M) and (V), and in the player M is Mute and V does nothing. `manual`
- Closing the captured window while on standby shows that the source closed and lets the screen be chosen again, with no stuck encoder. `todo`
- capture: a picture fits inside with even bars; an empty or tiny output never panics; colors become studio range bt709; a narrow picture gets black bars on the sides. `unit`
- data: the standby ring keeps the newest segments and only standby is trimmed. `unit`
- ffmpeg: choice skips dropped encoders down to the fallback; a dropped backend falls to another backend of the same codec; probe results are reused only for the same key. `unit`
- media: splitter finds init and whole fragments across reads; scan finds times and the last whole fragment; huge box sizes in damaged files stop the scan; patch moves times to zero even in partial ranges; segments cut on count and on set changes. `unit`
- record: speaking changes only when the mic turns loud or stays quiet past the hang; a window that sends no frames for a while is stalled; every start except the restart after an update has the microphone muted and keeps the volumes; recording without a screen records the voice only; a capture born before the clock moved is finished; presets scale down only and keep even sizes; bitrate follows pixels and codec; an encoder that takes frames but gives nothing is stalled; a write that never returns is a stall; no frames to encode is not a stall; output is stable only after a long working run; a restart logs each unlogged own segment once; strikes count one encoder in a row. `unit`
- room: sets come from rec and rec end records; records from other versions never break reading; an imported view carries its name importer and title in its first rec; set contents come from seg records; a new recording never takes an id in use; only a recording in progress or a requested one is received; log lines carry the local wall clock; restore and caps; coverage by moment or set; a stop in any log ends the set and a crash lets its member rejoin; a recording ends for the list once nobody present records it. `unit`
- everyone is told who started or stopped recording; the screen and microphone buttons share one state: off, on or a warning; the microphone button shows a live level that sits flat while muted. `ui-test`

## Live

- live segments flow and C gets the history. `e2e` (`live`)
- Live shows a tile for everyone who shares a screen, also while recording. `e2e` (`live`)
- a member without a picture, on Live (offline or not sharing) or in a recording (only voice parts), gets a small avatar chip with the speaking ring and the mic-off mark in a strip under the tiles instead of a tile; the grid, the view buttons and their number keys count only members with a picture, the volume popup still lists every member (voice only for a chip), and with nobody pictured the chips sit alone in the middle. `ui-test` (who gets a chip), `manual`
- connections stay up while live tiles stay in step, a member stalls, and the others stop waiting for one too far behind. `e2e` (`live`)
- every tile (also the minimap tile) has its name strip above the picture, never over it, and the grid counts the strip in each tile height; a long name is cut with an ellipsis. `manual`
- dragging a name label swaps two tiles. `e2e` (`library`)
- Your own tile shows and moves right after recording ends and while it records. `todo`
- net: a live reply names the recording of each series; history gets its share while live fills the limit. `unit`
- record: a segment opens only beside its init; a live stream serves only segments of the recording it asked for and ends once it is two segments behind; quality steps down only while every direct receiver fell behind twice recently. `unit`
- History to one member waits while live flows to another but not while that live write is stuck, and in standby the quality drops one preset when live falls behind for every receiver. `manual`
- receiving speed and the live edge; live tiles wait for the slowest member, never go back, stop waiting for one too far behind, mark it late once passed, and advance while nobody holds; the live delay shrinks slowly only while the slack is above the gap, and a tile with no start follows its own edge; a Live tile tells at most one problem, only while this PC records; live lists every room member in the saved order and then in join order, whatever their connection, with a tile only while online and sharing, and no live file from a member reached through a relay. `ui-test`

## Playback

- full screen in the player and on the board is left like any video player: Esc (after a menu, dialog or popup has closed first, and a text field has been left), F, F11, double-click on a picture (not on the controls), the control bar's full screen button (pressed while on), and an Exit full screen button at the top right (tooltip Exit full screen (Esc), at least 32px, above popups, clear of the Watch Party pill) that shows when the mouse moves and fades after 2.5 s of stillness unless hovered or focused, together with the controls and the mouse cursor. `manual`; which key does what `ui-test`
- recording alone, the player follows the picture as it grows. `e2e` (`live`)
- each ended recording gets a thumbnail, and the sidebar lists recordings with it. `e2e` (`library`)
- A recording in HEVC and in AV1 plays on every PC; a PC that cannot play one makes the room choose another codec. `manual`
- a far seek and an immediate Play start every tile at once, alone and in Watch Party; a tile whose recording began within a second of the seek is not counted as apart. `e2e` (`library`)
- playing your own recording right after it ended shows your own picture, moves on and reaches its end. `e2e` (`recover`)
- stream: a recording folder lists its pieces from the lowest segment and the live folder its newest run, each with first times and an open tail, served whole or from a byte, and the index from a segment on; a recording plays from its own folder up to the first missing segment; ids are checked and paths decoded. `unit`
- room: a part starts when its capture began, with the listing's size and whether its last piece is still open. `unit`
- thumbs: the middle segment holds half of the length; the picture is scaled to cover and cropped at the centre to exactly 16:9, whatever the source shape. `unit`
- the player plans the segment holding the moment and the next ones, continues a grown tail, ends a part where its last piece ends, and holds every tile until all can play. `ui-test`
- the player fetches a growing recording's index from its last known piece on, and the whole index again when that piece is no longer there. `ui-test`
- a focused player plays its own sound; otherwise the sound is the one chosen by a click, else the viewer's own, else the first player's; the view line names the first member who blocks the playback, in one sentence, and a holder reached through a relay does not count. `ui-test`
- every avatar outside dense lists (sidebar faces, board chips, dialogs, dock, tiles, voice chips) carries its member id and shows the speaking ring in that member's colour (with a glow and a light pulse from 24px up, unless motion is reduced), except the quiet ones in dense lists (members table, notes, watcher pills, volume popup, diagnostics); an avatar rebuilt while the member talks is lit at once, and your own muted mic lights nothing. `ui-test`, `manual`
- live speaking leaves the player alone and a played voice lights its tile: a live `speaking` message lights no player tile and every voice moves with its media through Web Audio (`e2e`, `library`); the ring following a real voice and fading after the hang (`manual`); a voice counts as talking while loud and for the hang after (`ui-test`).
- clicking a tile, or Switch app sound in its right-click menu, picks whose app sound plays and clicks keep no focus: the chosen tile is heard and marked, the view buttons show their shortcut numbers up to the sixth member, the app sound button opens one popup with each member's app sound and voice volumes and a button that picks whose app sound is heard; speed is a button in the transport row, hidden while following Live, layout is the last button of the view switcher, and after clicks on the volume and play buttons Space and the arrows still work (`e2e`, `library`).
- the Apex minimap extension, off by default and chosen per room on the Extensions page without a runner, analysis or job, adds a tile in the All view that redraws the top-left minimap corner of the heard member's video (scaled from the frame height and anchored at the top left so it holds for 720p to 4K and 16:10, 21:9 and 4:3); it takes the free cell of the grid or adds one, follows the heard member, and is not shown in a focused view, while that video has no picture or when its cell is under 96 px high; the crop is redrawn when the video size changes and drawn at the screen's pixel ratio. `ui-test` (the crop at capture sizes from 720p to 5120x1440, 4:3, portrait and square, as recorded at each quality width), `unit` (no job), `manual` (layout at 2, 3 and 4 members, pixel ratio 1 to 2)
- repeated seeks in one direction add up and a change of direction starts over (`ui-test`); the seek bezel shows the sum, for example 15 s back after three quick left arrows, and releasing a seekbar drag shows the target time in the centre bezel (`manual`).
- a voice counts as speaking until the hang has passed and not after (`ui-test`); the bezel and a run of seeks end after the bezel time even with animations off (`manual`).
- the page scripts load as one scope without a name declared twice. `ui-test`
- dragging the volume and the popover ranges still works with clicks not taking focus; Tab still reaches the controls with a visible focus ring. `manual`
- The recording row and Storage counts (notes, matches) leave out drawings and deleted or erased notes and redraw as soon as one changes. `ui-test`
- The player side panel is the timeline only, one line per row (time, text, author dot) in time order with match starts as rows and no row for drawings (they stay on the video), a click anywhere on a row (or Enter or Space on the focused row) jumps to its moment except on a button inside it, over a text selection or at the end of a drag, editable in place, a right-click on a row opens a menu with only what the row can do (Go to this moment, Copy text, Copy link to this moment, and for own notes Edit and Delete with the Undo toast; marks and extension events get no Edit), a note added through the api shows with the AI mark; no tabs, search or scope. `e2e` (`api`)
- Each extension that applies to the open recording has a row in the Analysis section above the debriefs, styled like a timeline row under a header with an icon and a count: a round status icon, the extension name, a state pill (Done, Analyzing with the percent and a progress bar, Waiting, Failed, Not analyzed), a muted line with the reason or whose PC runs it, and Retry or Analyze for failed and not analyzed rows; the debrief rows show the length then when with Save; either section is hidden when none applies; right-click on an Analysis row opens Details (extension and version, state, reason, runner, recording, finish time, progress, results, error; Copy copies it all), Run again (not while running, disabled while waiting) and Copy error (failed with an error); the recording row menu in the sidebar has no analysis action; the sidebar rows show no analysis mark except a small glowing chip with the Analyzing tooltip (and percent) while any extension runs on that recording, static when motion is reduced. The Analysis and Debrief recordings sections sit in their own region under the timeline that scrolls on its own and takes at most about half the panel. `ui-test` (state text), `manual` (layout)
- The debriefs of the open recording are listed at the bottom of the side panel under Debrief recordings, one row each with start time, length and Save, which exports only that one debrief through the folder export; the section is hidden with none and the header has no debrief chip. `e2e` (`library`)
- the recorded voices in the player follow Recorded voices in Settings > General, a side (Auto, Left, Right or Center) and a tone (Clear, Mild or Strong), remembered on this PC and Auto and Clear at first; Auto is Left in a Watch Party with a recording open and Center otherwise; Center and Clear plays them as recorded, their volume never changes, and the app sound and Live are never touched. `ui-test`
- the app sound popup ends with Recorded voices position and tone…, which closes it and opens Settings > General with the focus on Recorded voices. `manual`
- listening, the recorded voices move to the chosen side (to the left on joining a Watch Party with Auto, back to the middle on leaving), Mild softens them and Strong thins them more, a change in Settings is heard at once, and the app sound and the live talk stay as they are. `manual`

## Watch Party

- the app starts outside Watch Party, and joining is the Join button. `e2e` (`library`)
- the Watch Party pill sits at the top right of the picture and shows who it is catching up with, and a recording on this PC shows a Watch Party button on its second line on hover that opens it and joins, with nothing in the row moving when it appears; while members watch it the row always shows their small faces in an accent pill with Join, and Watching once this PC is in that party; the name stays readable and the pill moves to a second line when it must; the recording in progress shows the faces of the members watching it the same way, and no participant faces on its picture. `manual`
- Watch Party converges on a seek and a play, and a member who joins midway converges too. `e2e` (`library`)
- leaving and joining the Watch Party, going to Live and back, and the starter leaving keep everyone's place: leaving keeps the position, playing and layout; coming back opens the recording where it was left; a late joiner follows the party and moves nobody; the party goes on without the one who started it. `e2e` (`library`)
- a party state is applied once, whatever its count compared with one from another recording; joining moves the player quietly; a peer's play, pause or far seek is announced and a small drift is corrected silently. `ui-test`
- a recording opened again in the same run starts where it was left, also after Live, a recording start or leaving the room; a moment link to 0:00 still goes to 0:00. `e2e` (`library`) for Live and back, `manual` for the rest
- stopping a recording opens it from the start for everyone in the Watch Party, and leaves a PC already watching that recording where it is. `e2e` (`library`)
- a session started while watching together records voices and the Watch Party; ending a debrief moves nobody, not even a PC on Live; while it records, the start box says Stop debrief and its confirm asks to stop the debrief recording. `e2e` (`library`)
- the start button reads Record debrief while this PC watches a recording together, and Start recording otherwise. `e2e` (`library`)
- A real Watch Party of a recording from several PCs stays in step through seeks and pauses. `manual`
- party: newest wins by seq then time then origin; simultaneous actions converge; a joiner adopts the running state; turning on joins the party without moving it; rejoining takes the running state again; party messages for another recording are not followed; watching lists the viewers per recording and includes me while in the party; positions move to the receivers clock; heads only grow and show who is behind; typing shows while the viewer is seen; other rooms and off are ignored but viewers are counted; a review starts only while watching a session together; party records come back on the local clock in order; a late or restarted joiner never moves the party back; leaving moves nobody and the party goes on without the one who started it; a stale state from a reconnecting member is ignored; the first action after joining wins over the state heard before; opening another recording takes its party even with a lower count. `unit`

## Debrief

- a note with a tag, its edit and its deletion reach everyone. `e2e` (`live`)
- the game plan is a board of cards with short Markdown, a tag chip on a card opens a recording with its timeline filtered by that tag, and a card is added, renamed, moved, resized and deleted by anyone. `e2e` (`library`)
- on the board, dragging a card's corner resizes it in whole columns and its height as a minimum with an outline, dragging its header shows a bar where it will land, a card grows to show all its text, and the team card resizes but does not move. `manual`
- the board shows every card in full with no scrollbar: cards pack densely in 12 columns and are as tall as their text, the text starts near the body size, grows only a little on wide boards, and shrinks in half pixels until the board fits, never below 10px. `unit`
- the board fits again after a window resize, a card resize or move, and full screen; its bar has only Add card and a More menu with Board window and Full screen, and F still toggles full screen. `manual`
- the Board window is an ordinary window with a title bar and the app icon, 1100x760 and centred on first open and then where it was left, at least 480x320: it moves, resizes with the board fitting again, and closes with the title bar X, Alt+F4 and Esc (Esc only leaves full screen or the card field while those are on) without closing the main window; F toggles full screen in it and a reopen starts windowed. `manual`
- the sidebar width is dragged, kept, and never scrolls sideways. `e2e` (`library`)
- jobs: room events hide unfinished plugin runs. `unit`
- room: tags are read from text; @names are stored as member ids with the written name (a duplicate name stores each member, an old note stores none); events merge with their edits; peer events are cleaned. `unit`
- match ends come only from plugin moments tagged match; matches run between consecutive ends, with the open one only while recording; a match end reads Match n end for the match it closes and its chapter Match n, in English and Japanese, and a plugin's match tag shows no chip while other tags do; a and D land a minute before the previous or next match end; Q and E step through notes and marks that have a time, in order; progress shows percent, speed and time left and drops what is unknown; strokes drawn at one moment by one person are one row; a recording is hidden from the list when it is a debrief of another, and debriefs are found by the recording they name; the sidebar width stays between limits and the splitter takes the arrow keys; the one drawing button shows the pen or the laser and cycles Pen, Laser, off on a click and on P, pressed while a tool is active, with a tooltip of name, one line and key in English and Japanese; the keys have no duplicate among the player keys and are listed in the shortcuts; markdown reads headings, lists and paragraphs only; markdown inline reads emphasis, code, links and tags, and only safe links become links; a card is a bold first line over short markdown, with marked text, scene links and tags; dropping a card between two others takes the middle position, and at the ends one step beyond; a resized card snaps to whole grid cells within the limits; a dragged card lands before the card under the pointer's left half, after its right half, and in reading order between cards; a card shows nothing a note can write as running or navigating; a moment link keeps a formatted label, and deep lists render without running out of stack. `ui-test`

## Extensions

- an Apex match detection job for the set is claimed once and finished. `e2e` (`library`)
- the runner of an extension is chosen and a rerun is processed there. `e2e` (`library`)
- the runner of an extension is chosen from the members in its row. `e2e` (`library`)
- jobs: a new config record keeps old results and reruns nothing; the latest config wins on the shared clock; the runner defaults to who turned it on; only the runner processes and a runner who left is no runner; changing the runner keeps it on and stamps the shipped version; the runner wants every finished file of an enabled extension; the runner wants only files that still need analysis; only a recording that ended within the hour or was already started is analysed on its own; a recording with one file running reads running and a failure outranks it; stop and retry records read as pending; a file nobody online holds fails as missing and only an offline runner waits; a recording shows its chosen file and only enabled extensions; a change made here beats a record from a fast clock; keys hash the empty settings of older versions and change with the version; the newest recording comes first and an interrupted own claim before it; nothing runs when off; a built in extension runs the shipped version whatever the room recorded; turning an extension on stamps the shipped version; a waiting file tells what holds it with the checks the runner uses; an input is ready only with its whole picture and voice; done and failed files wait for a retry; a job stopped twice without finishing fails and a stop on purpose does not count; a plugin reads the longest file once; restarted captures of one person are one file; results of a plugin the app does not know are not shown; a plugin the app does not know runs nothing and shows no analysis; a file done twice shows the newest result and ties go to the lowest owner; results wait for the done record within caps; re running asks every enabled extension or the one named for each counted file; a retry after a failure on an older version runs the shipped one; results land on the runners clock; a capture spans its segments on the local clock. `unit`
- a link's text vouches only for the host it names; an extension row says one thing: no runner, what failed, who it waits for, or what is left; an Analysis row waiting on the runner's own PC says it waits for the recording to end or for the video tools. `ui-test`

## Storage and export

- the set exports to a folder of plain files lined up on its start. `e2e` (`library`)
- one member's light export into a named folder holds only that member's files, the next folder of that name is numbered, and a per-match export of a recording without matches is refused. `e2e` (`library`)
- deleting a recording others still hold takes its debrief along and keeps both as not on this PC until Download. `e2e` (`library`)
- deleting a recording nobody else holds removes it and its entry everywhere. `e2e` (`library`)
- a member cut off for a whole recording lists it as not on this PC and fetches it only when asked. `e2e` (`library`)
- A folder export opens in a video editor with every file lined up on the recording start. `manual`
- Export to folder… and a debrief's Save open the export dialog with Save to and the full path: a new folder named after the recording (title or start time, then the room) or the debrief (room, debrief, start time, length), in Videos\EZ Hanseikai at first or the folder last chosen with Change… on this PC, numbered when taken; Export starts it with no folder picker, the row's tooltip is that path and the finished toast has Open folder. `manual`
- data: hours left come from the newest sessions. `unit`
- export: one input without delay is copied and the rest is filtered; the filter places each capture on the timeline; the encode arguments map only what exists; the folder export always starts every file at the recording start; events become notes and chapters on the recording clock; the export is sized and paced by its sources; file names are safe and distinct; exported file names never meet the files beside them; a match takes only its segments and cuts into the first; per match files are named by match number and start and never copied; quality sets the picture size and bitrate; notes are written as csv; webvtt is written with numbered cues; finishing replaces the destination and leaves no part; folder names are safe on Windows (reserved names such as CON and NUL, trailing dots and spaces, long and Japanese names); a taken folder name gets a number. `unit`
- log: append scan and repair a torn tail; extend takes whole records at the right offset only. `unit`
- main: the docs name every record kind and message the code writes; notes keep line breaks and tabs and drop other control and direction characters; json files may start with a byte order mark; whole writes replace the file and leave no part. `unit`
- room: a recording is removed completely only when no other member holds a part; deleting a recording removes only its folder and thumbnail; a recording removed completely takes its results and jobs with it; deleting a recording deletes only debriefs that name it and keeps peers copies. `unit`
- the delete preview lists every part that is here and says whether the recording goes completely; a row shows its size or a dash when the recording is not here, and deleting needs media here or nobody else; the two delete confirmations read the same in both languages; the clean up review picks every group and each old folder without an ID key, and totals what is picked, in both languages; an imported recording is named by its title and the others by their time; the folder export is a menu action of the page; the export options fall back to the defaults and to everyone for a member not in the recording, and send no member for everyone; the export dialog shows the full path it saves to, offers the options only for a recording with a picture, and waits for the path. `ui-test`

## Updates and migration

- C keeps every file it does not know and drops only its standby segments. `e2e` (`library`)
- C's Clean up lists, then moves only old files it made that nothing refers to to the Recycle Bin and deletes its temporary files. `e2e` (`library`)
- Files that Clean up moved come back from the Windows Recycle Bin with Restore, and the app reads a restored recording again. `manual`
- cleanup: the plan lists only known app files that nothing refers to with one item per whole folder, in groups sorted by size; a run moves exactly the plan through the mover and keeps everything else; a run checks again before each move and refuses while busy; old folders go whole only by exact name with known entries and never the current ones, a folder with an unknown entry is kept and listed, and one holding key.bin is flagged; every pattern names what it removes. `unit`
- The Clean up review groups what it would move with sizes, largest first, expands each group to its paths, starts with an old folder that holds an ID key unticked, and moves only what is ticked. `manual`
- The installer run over a running app closes it, installs and starts the new version with the same rooms and recordings. `manual`
- The in-app update installs the new version, also on a PC that was on the Beta channel, ends your part first and cancels a running export, and while recording the app comes back into the recording in a new part. `manual`
- The old repository address still reaches the update manifest and the installer download. `todo`
- data: one app holds a data folder and a leftover lock is noticed; a reset keeps the logs, and the rooms only when asked; a restart removes nothing under recordings; counts come from the files present with their logged size. `unit`
- main: settings from older versions fall back to defaults and keep only https relay URLs; rooms from older versions keep what still reads. `unit`

## API

- the local API is on from launch, reads recordings and takes a note and a game plan that everyone sees, and the page switches it off and on. `e2e` (`library`)
- api: only the loopback address and port are a valid host; the token must match exactly; requests are read with their query and body and paths decode strictly; malformed and oversized requests are refused; a wrong host or token gets nothing and llms txt needs no token; rooms list their members and skip pending ones; a recording whose open file belongs to an absent member reads as ended; recordings come newest first with members length and kind; events have times from the recording start tags and authors; chapters are the matches between match ends; the board is the newest team card and can be replaced; a note added through the api is an ordinary event with its tags; bad events are refused without writing; an imported recording shows its title importer and views; an operation is called by name with its json arguments; unknown paths and methods are refused; plugins show their runner and what is left and recordings their analysis; a debrief names its recording carries the watch and serves its voices; the app info names the open room; status tells the state of this pc and every member; diagnostics give this pcs report or everyones; every reference entry has lines and the main endpoints are listed; the address file holds the address and token for this user only; llms txt describes every endpoint. `unit`
- Deleting the AI notes, retrying a job or exporting a recording of a room that is not in the list is refused as not on this PC. `todo`
- ops: the page calls only listed operations and every operation is documented; arguments are read by name; displays get their size and windows do not. `unit`
- the AI button in the sidebar bottom row, next to Settings, opens the AI menu above itself and passes the open recording, or the newest; the AI menu offers a few prompts, or the switch when the API is off; a prompt names the API address, the token, llms.txt, the room and the recording; endpoint examples that are JSON are shown as indented code and the rest as text. `ui-test`
- The AI prompt dialog shows the whole prompt in one editable box (task, blank line, then the address, masked token, room, recording and keep-the-token-secret line); Copy copies the box with the mask replaced by the real token (a deleted mask is copied as is); only the part before the first blank line is kept per purpose, so the facts are always fresh; one line above the box says the app does not run the AI and the prompt goes to the owner's own agent; "Reset to default" is enabled only when the task differs and restores the default. `ui-test` (mask swap, saved task, cleared task), e2e prompt step

## Network and clocks

- B's time answer runs 8.2 s ahead and C gets none, and the recording still lines up. `e2e` (`live`)
- A, B and C record the same set, C joining late only through the relay: it sends and gets voice and logs but no picture, and catches up once it comes back on a direct path. `e2e` (`recover`)
- a relay server from the settings is tried first and a dead one is skipped. `e2e` (`library`)
- measuring the speed to a member gives up and down in Mbit/s, the round trip and the path. `e2e` (`library`)
- B loses its network while everyone records, keeps recording its part, and everything reaches everyone once it is back. `e2e` (`recover`)
- Members on different app versions in one room keep working and ignore what they do not understand. `todo`
- clock: a request asks as a client and only a server answer to it is read; the offset is how far the server runs ahead in unix time; a clock that moved since the measurement keeps true time and asks again past a step; a dead server fails within one timeout. `unit`
- ffmpeg: download retries back off up to a limit; pace gives speed and time left. `unit`
- net: a fetched copy is kept only when it matches its record; only a run of failed connects is reported once; a ping reply is eight bytes and an empty one is no answer; a pull holds at least one whole record; own log is restored only when no member holds more; redials back off with jitter up to a limit; the connection dialed by the lower id is kept on both sides; speed is bytes over time in Mbit/s with the path at the end. `unit`
- room: logs are shared only with members of that room. `unit`
- relay: custom relays come first, each alone, and the public relays last together. `unit`
- the connection badge shows a relay when any member uses one; where data goes says when a relay or a server is involved; the new Diagnostics texts have Japanese translations; a download shows its percent, speed and time left. `ui-test`

## Robustness

- a second app on A's data folder exits and leaves A running. `e2e` (`live`)
- C loses its own log and gets all of it back before writing again. `e2e` (`live`)
- B crashes mid-recording and recovers its open segment. `e2e` (`recover`)
- B restarts during the recording as an update does: the recording goes on and B is back in it with its screen in a new part. `e2e` (`recover`)
- A's encoder is killed and A keeps recording the set in a new part. `e2e` (`recover`)
- closing the captured window during the recording goes on with the voice, and a new screen adds a part. `e2e` (`recover`)
- C's disk is full during the recording: its part ends with a notice, the others go on, and C completes its copy once there is room. `e2e` (`recover`)
- Diagnostics opens on where data goes and copies a masked report. `e2e` (`library`)
- The first start shows the Windows firewall prompt and the app works after Allow. `manual`
- The first start downloads ffmpeg, checks it and keeps retrying with no network. `manual`
- The window, menus, dark theme and Japanese texts look right at normal and high DPI scaling. `manual`
- every menu and popup opens against the edge of what was clicked and stays inside the window: the room menu, the AI menu, the microphone and screen carets, the Layout and speed menus and the app sound popup touch their button on the side asked for and flip when that side is short, and right-click menus on a tile or a recording row open at the pointer. `e2e` (`library`); the placement for any button, window size and popup size. `ui-test`
- the menu bar, with the line under it, is the same near-black as the title bar and the app on a fresh start, after a language or shortcut change (the menu is rebuilt) and after full screen (also on a PC set to the light theme), its drop-down menus are dark, and the menu bar is hidden while the player or the board is full screen and back after. `manual`
- three members record for an hour with Live open: the app's private memory and CPU stay flat after the first minutes and the heap keeps no growing free space (Windows segment heap). `manual`
- an hour-long recording is counted, checked for missing pieces and sized with one listing per folder, not work per segment, so a long session does not slow the app or grow its memory. `unit`
- check: a line starts with its level. `unit`
- debug: the transfer list keeps the newest; kinds follow live series and init; a piece after a failure is a retry; a hash failure marks the last receipt or adds a row; jobs are grouped with their latest state; reports hide addresses ids and names; reports hide a user folder with spaces; a job reads as one line; addresses are hidden and the rest kept; errors keep the newest warnings; the tail starts at a whole line; the flow text names each route and hides hosts; a combined report names each member by short id. `unit`
- hw: hardware is read from the script output; bad output gives empty hardware; the encoder names its kind and the matching gpu; the report line names the hardware encoder and capture rate; a healthy recording logs nothing until it ends; an idle screen is not a collapse; a stalled encoder is logged once until it recovers; short or voice only recordings end quietly; slow sends are logged rarely. `unit`
- shell: colors are written blue first for windows; translations are unique and fall back to english; the start command says what it records; only plain web links open; joins within a few seconds make one notice naming two and counting the rest, none while the window is watched; the join notice sits at the bottom right of the work area at any scale; the join chime is a short mono wave; the hotkey needs ctrl or alt; a shortcut another app holds is reported only while armed; saved hotkeys load and move from an old default to the new one; closing asks only while work would be lost; clocks read like the player. `unit`
- sources: a closed window is never replaced by another window of the same program. `unit`
- a time of day always shows its date (Oct 23 00:01, 10/23 00:01, with the year only when not this year) and the debrief and recording rows put the length first and the time muted after a middle dot; shortcuts need Ctrl or Alt and read like the native menu; the global shortcut row is named and can be reset only when it differs from its default; the global shortcut another app holds shows its row in a warning state; the pages call a recording a recording, never a session; every English phrase and fixed message has a Japanese one and every Japanese entry is used; every end-to-end step is named in docs/test-cases.md; Settings has the Notify when someone joins switch, kept in the app's settings. `ui-test`
- start page: an update or video-tools notice shows before any room is open; the two main actions read Join a room and Create a room. `manual` (UI walkthrough harness)
- notes: #tags and @member names show as chips wherever they appear (timeline, notes on the video, game plan cards); an @member chip that resolves to exactly one current member starts with that member's small avatar and one that does not has none; a chip filters the timeline of the open recording and shows the panel, from elsewhere it opens the last open or newest recording filtered, a game plan card matches mentions by name, and in the player a second tap or the × clears it; typing # or @ suggests tags or names and Enter, Tab or a click inserts the pick; a name matches longest first, with spaces, Japanese and emoji, and never inside a word; a stored mention follows a renamed member (chip shows the current name) and filters by id, an old note without ids filters by name. `ui-test`
- interface text (labels, buttons, menus) cannot be selected, while notes, events, cards, tables, fields and code can; the browser menu never shows and right-click offers Copy, Cut and Paste only where text is selected or editable. `manual` (UI walkthrough harness)

## User input

The UI cleans typed text and the app cleans it again, the same way, and counts whole characters (the page's `maxlength` counts UTF-16 units, so it never allows more than the app keeps). Limits: name 24, board card title 24, room name 40, recording title 80, note 500, board card 4000, invite code 1024, relay URL 256 and 8 URLs, tags 16 per API note.

| Input | Expected result | Checked |
| --- | --- | --- |
| every text field is typed to its limit | the field stops at the limit; every page that shows it keeps its layout (ellipsis with the full text as tooltip on one line, wrapping on several) | `manual` (UI walkthrough harness, worst-case scenarios `w01` to `w22`) |
| the page limits equal the app limits | one `LIMITS` object in `ui/util.js` equals the Rust constants | `ui-test` |
| empty, whitespace-only, zero-width or direction-only name, room name, note | refused or reset to the old value, never saved blank | `unit` (main.rs), `ui-test` |
| leading and trailing spaces, line breaks and tabs in a one-line field | one trimmed line, breaks become spaces | `unit`, `ui-test` |
| control, zero-width, soft-hyphen and direction characters | removed | `unit`, `ui-test` |
| emoji, joined emoji, East Asian full-width text | cut at the limit in whole characters, a joined emoji stays whole, the initial in an avatar is a whole character | `unit`, `ui-test` |
| a letter with 30 combining marks | at most 3 marks stay | `unit`, `ui-test` |
| HTML, Markdown or a script in a name, note or card | shown as text, never run | `ui-test` (card shows nothing a note can write as running), `manual` |
| a note or card pasted far beyond its limit | the field keeps the first characters; the app and the API cut or refuse the same | `unit`, `manual` |
| an invite code with spaces, line breaks, quotes, capitals, zero-width characters or text around it | joins the same room | `unit` (room.rs) |
| an invite code that is empty, 5000 characters, has no token or is not hex | "That invite code is not valid." and nothing changes | `unit` (room.rs) |
| a relay URL that is not https, longer than 256 characters, or more than 8 URLs | refused with a short message | `unit` (relay.rs) |
| upload limit of 1e300 or below 0 | capped at 100000 Mbit/s, or no limit | `manual` |
| Enter or a click twice in a row on Create room, Join a room, Save room, a note or a card | one room, one join, one rename, one note, one card | `ui-test` (a running submit ignores the next one) |

## Interface

- every inline text editor (quick note, timeline note, board card and new card name, start name, room settings) has an x button at its corner with the tooltip Cancel (Esc), Esc cancels, Enter (Ctrl+Enter in the card text) saves, and clicking outside never drops typed text: a note, card name or start name keeps or saves it, and Room settings stays open while it has unsaved changes. `ui-test` (the keys), `manual`
- the timeline time column fits its longest time with no overlap, in English and Japanese at 820 and 1280 wide; a recording not on this PC dims only its thumbnail and keeps its text readable; small buttons, the connection badge, the Join pill and Download are at least 28 px tall to click; high contrast mode outlines the selected and pressed states. `manual` (UI walkthrough harness)
- a last seen time has the full local date and time as a tooltip; IDs show three groups everywhere with the full ID as a tooltip; the window is named after the room, and the recording open in the player. `ui-test`
- the app reopens the room's last page or recording, with its timeline scroll, after a restart. `manual`
- Extensions and Settings > Recordings show Loading before the first answer, an empty state, and a failed load with Retry. `manual`
- deleting a board card names it in the confirmation and can be undone; Close room asks first while a recording runs, and the start screen's Delete room… keeps its warning. `manual`
- ? opens the shortcuts on the start screen too; the player tooltips and the View menu show their keys; a confirmation that deletes focuses Cancel and the others focus their action; a popup gives focus back to its opener; the toast and the player messages are announced. `manual`
