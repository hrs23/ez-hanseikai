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
- the room header menu lists the room commands and a rename in its settings reaches everyone. `e2e` (`library`)
- each member row says one short state. `e2e` (`library`)
- B's chosen colour and built-in avatar and A's room icon reach everyone. `e2e` (`library`)
- A real two-PC test with friends: create a room, invite, join, approve, and see everyone's name, avatar, color and state. `manual`
- Picking a photo in Settings crops it to the middle and shows on the other PC. `manual`
- Pasting your own invite code is refused as not valid. `todo`
- A member dialing someone who left the room logs that the other app closed it as not a member. `manual`
- room: joining waits for an answer and ends the invite; the latest remove or admit wins; the latest room name wins on every clock; a rename made here is stamped one after the newest name known, even from a fast clock; invite tokens and room ids cannot leave the rooms folder; a peer can only name a room with an id this app makes; a member is present while connected or seen within a minute; colors are the same everywhere; colors stay when someone joins without a clash; a chosen colour replaces only its owner's colour and is the same everywhere; the newest profile in a member's own log gives their colour and avatar and a bad colour, an svg or an oversized image is skipped; a profile is written only when the look differs; writes go only to the open room once joined, never to a pending one; the newest room icon wins on every clock and an empty one clears it; settings drop a bad colour and avatar; an old debrief links through its first party record; ai del hides ai notes from every log but not human notes; board cards newest event wins and old team card reads. `unit`
- Room settings: picking a preset sets the icon to a PNG image of it; Remove returns to the id-picked default; an avatar stored under an old built-in name (bolt, star, heart, diamond, shield, moon, target) shows its replacement glyph. `manual`
- a member row says one short state, also for a member reached through a relay; an offline avatar is dimmed whatever it shows; Settings shows a picked image as a pressed tile; an avatar shows the image, a built-in shape or the initial, an unknown shape shows the initial, a room without an icon shows a preset glyph and colour picked from its id (the same on every PC), and a room with no id shows its initial. `ui-test`
- the avatar stack shows +n for the faces it cuts. `manual`

## Recording

- Record on A starts the set on B. `e2e` (`live`)
- while recording the start box says Stop recording with the time and the list gets the running recording. `e2e` (`live`)
- the set stops and everyone has it. `e2e` (`live`)
- A, B and C record a short set, C joining late and recording long enough that every tile has video at the far seek. `e2e` (`library`)
- Start pressed on A and B together makes one recording on every PC. `e2e` (`library`)
- the microphone, the mix and the hotkey are kept in the settings. `e2e` (`library`)
- A real two-PC recording of a real game with game sound and the microphone, lined up in playback. `manual`
- A long recording of an hour or more with three members runs without a stall, a growing delay or a lost part (soak test). `manual`
- The hardware encoders (NVENC, AMF, QSV) record on each GPU kind, and the fallback to the software encoder works. `manual`
- The global hotkey starts and stops a recording while a game has the focus. `manual`
- Closing the captured window while on standby shows that the source closed and lets the screen be chosen again, with no stuck encoder. `todo`
- capture: a picture fits inside with even bars; colors become studio range bt709; a narrow picture gets black bars on the sides. `unit`
- data: the standby ring keeps the newest segments and only standby is trimmed. `unit`
- ffmpeg: choice skips dropped encoders down to the fallback; a dropped backend falls to another backend of the same codec; probe results are reused only for the same key. `unit`
- media: splitter finds init and whole fragments across reads; scan finds times and the last whole fragment; huge box sizes in damaged files stop the scan; patch moves times to zero even in partial ranges; segments cut on count and on set changes. `unit`
- record: speaking changes only when the mic turns loud or stays quiet past the hang; a window that sends no frames for a while is stalled; every start except the restart after an update has the microphone muted and keeps the volumes; recording without a screen records the voice only; a capture born before the clock moved is finished; presets scale down only and keep even sizes; bitrate follows pixels and codec; an encoder that takes frames but gives nothing is stalled; a write that never returns is a stall; no frames to encode is not a stall; output is stable only after a long working run; a restart logs each unlogged own segment once; strikes count one encoder in a row. `unit`
- room: sets come from rec and rec end records; records from other versions never break reading; an imported view carries its name importer and title in its first rec; set contents come from seg records; a new recording never takes an id in use; only a recording in progress or a requested one is received; log lines carry the local wall clock; restore and caps; coverage by moment or set; a stop in any log ends the set and a crash lets its member rejoin; a recording ends for the list once nobody present records it. `unit`
- everyone is told who started or stopped recording; the screen and microphone buttons share one state: off, on or a warning; the microphone button shows a live level that sits flat while muted. `ui-test`

## Live

- live segments flow and C gets the history. `e2e` (`live`)
- Live shows a tile for everyone who shares a screen, also while recording. `e2e` (`live`)
- connections stay up while live tiles stay in step, a member stalls, and the others stop waiting for one too far behind. `e2e` (`live`)
- dragging a name label swaps two tiles. `e2e` (`library`)
- Your own tile shows and moves right after recording ends and while it records. `todo`
- net: a live reply names the recording of each series; history gets its share while live fills the limit. `unit`
- record: a segment opens only beside its init; a live stream serves only segments of the recording it asked for and ends once it is two segments behind; quality steps down only while every direct receiver fell behind twice recently. `unit`
- History to one member waits while live flows to another but not while that live write is stuck, and in standby the quality drops one preset when live falls behind for every receiver. `manual`
- receiving speed and the live edge; live tiles wait for the slowest member, never go back, stop waiting for one too far behind, mark it late once passed, and advance while nobody holds; the live delay shrinks slowly only while the slack is above the gap, and a tile with no start follows its own edge; a Live tile tells at most one problem, only while this PC records; live lists every room member in the saved order and then in join order, whatever their connection, without a picture from a member reached through a relay. `ui-test`

## Playback

- recording alone, the player follows the picture as it grows. `e2e` (`live`)
- each ended recording gets a thumbnail, and the sidebar lists recordings with it. `e2e` (`library`)
- A recording in HEVC and in AV1 plays on every PC; a PC that cannot play one makes the room choose another codec. `manual`
- a far seek and an immediate Play start every tile at once, alone and in Watch Party; a tile whose recording began within a second of the seek is not counted as apart. `e2e` (`library`)
- playing your own recording right after it ended shows your own picture, moves on and reaches its end. `e2e` (`recover`)
- stream: a recording folder lists its pieces from the lowest segment and the live folder its newest run, each with first times and an open tail, served whole or from a byte, and the index from a segment on; a recording plays from its own folder up to the first missing segment; ids are checked and paths decoded. `unit`
- room: a part starts when its capture began, with the listing's size and whether its last piece is still open. `unit`
- thumbs: the middle segment holds half of the length. `unit`
- the player plans the segment holding the moment and the next ones, continues a grown tail, ends a part where its last piece ends, and holds every tile until all can play. `ui-test`
- the player fetches a growing recording's index from its last known piece on, and the whole index again when that piece is no longer there. `ui-test`
- a focused player plays its own sound; otherwise the sound is the one chosen by a click, else the viewer's own, else the first player's; the view line names the first member who blocks the playback, in one sentence, and a holder reached through a relay does not count. `ui-test`
- live speaking leaves the player alone and a played voice lights its tile: a live `speaking` message lights no player tile and every voice moves with its media through Web Audio (`e2e`, `library`); the ring following a real voice and fading after the hang (`manual`); a voice counts as talking while loud and for the hang after (`ui-test`).
- clicking a tile picks whose app sound plays and clicks keep no focus: the clicked tile is heard and marked, the app sound button picks the next member, and after clicks on the volume and play buttons Space and the arrows still work (`e2e`, `library`).
- repeated seeks in one direction add up and a change of direction starts over (`ui-test`); the seek bezel shows the sum, for example 15 s back after three quick left arrows, and releasing a seekbar drag shows the target time in the centre bezel (`manual`).
- a voice counts as speaking until the hang has passed and not after (`ui-test`); the bezel and a run of seeks end after the bezel time even with animations off (`manual`).
- the page scripts load as one scope without a name declared twice. `ui-test`
- dragging the volume and the popover ranges still works with clicks not taking focus; Tab still reaches the controls with a visible focus ring. `manual`
- The player has one Review tab: analysis status rows, then the search and scope; with no search words and this recording the notes, chapters, moments and drawings sit in their match (or under Whole recording without matches), editable in place; a search or all recordings gives the flat list; a note added through the api shows in it with the AI mark. `e2e` (`api`)

## Watch Party

- Watch Party converges on a seek and a play, and a member who joins midway converges too. `e2e` (`library`)
- stopping a recording opens it from the start for everyone in the Watch Party, and leaves a PC already watching that recording where it is. `e2e` (`library`)
- a session started while watching together records voices and the Watch Party; ending a debrief moves nobody, not even a PC on Live. `e2e` (`library`)
- A real Watch Party of a recording from several PCs stays in step through seeks and pauses. `manual`
- party: newest wins by seq then time then origin; simultaneous actions converge; a joiner adopts the running state; turning on joins the party without moving it; rejoining takes the running state again; party messages for another recording are not followed; watching lists the viewers per recording and includes me while in the party; positions move to the receivers clock; heads only grow and show who is behind; typing shows while the viewer is seen; other rooms and off are ignored but viewers are counted; a review starts only while watching a session together; party records come back on the local clock in order. `unit`

## Debrief

- a note with a tag, its edit and its deletion reach everyone. `e2e` (`live`)
- the game plan is a board of cards with short Markdown, and a card is added, renamed, moved and deleted by anyone. `e2e` (`library`)
- the sidebar width is dragged, kept, and never scrolls sideways. `e2e` (`library`)
- jobs: room events hide unfinished plugin runs. `unit`
- room: tags are read from text; events merge with their edits; peer events are cleaned. `unit`
- match ends come only from plugin moments tagged match; matches run between consecutive ends, with the open one only while recording; a and D land a minute before the previous or next match end; Q and E step through notes and marks that have a time, in order; progress shows percent, speed and time left and drops what is unknown; strokes drawn at one moment by one person are one row; a recording is hidden from the list when it is a debrief of another, and debriefs are found by the recording they name; the sidebar width stays between limits and the splitter takes the arrow keys; markdown reads headings, lists and paragraphs only; markdown inline reads emphasis, code, links and tags, and only safe links become links; a card is a bold first line over short markdown, with marked text, scene links and tags; dropping a card between two others takes the middle position, and at the ends one step beyond; a card shows nothing a note can write as running or navigating; a moment link keeps a formatted label, and deep lists render without running out of stack. `ui-test`

## Extensions

- an Apex matches job for the set is claimed once and finished. `e2e` (`library`)
- the runner of an extension is chosen and a rerun is processed there. `e2e` (`library`)
- the runner of an extension is chosen from the members in its row. `e2e` (`library`)
- jobs: a new config record keeps old results and reruns nothing; the latest config wins on the shared clock; the runner defaults to who turned it on; only the runner processes and a runner who left is no runner; changing the runner keeps the rooms version; the runner wants every finished file of an enabled extension; the runner wants only files that still need analysis; a recording with one file running reads running and a failure outranks it; old yield and stop records read as pending; a file nobody online holds fails as missing and only an offline runner waits; a recording shows its chosen file and only enabled extensions; a change made here beats a record from a fast clock; keys hash the empty settings of older versions and change with the version; the newest recording comes first and an interrupted own claim before it; nothing runs when off or mismatched; an input is ready only with its whole picture and voice; done and failed files wait for a retry; a job stopped twice without finishing fails and a stop on purpose does not count; a bundled plugin reads the longest file once and others count per file; restarted captures of one person are one file and old keys are ignored; old speech on a file that is not chosen stays shown; an old transcribe config runs nothing and shows no analysis; a file done twice shows the newest result and ties go to the lowest owner; results wait for the done record within caps; re running asks every enabled extension or the one named for each counted file; results land on the runners clock; a capture spans its segments on the local clock. `unit`
- a link's text vouches only for the host it names; an extension row says one thing: no runner, what failed, who it waits for, or what is left. `ui-test`

## Storage and export

- the set exports to a folder of plain files lined up on its start. `e2e` (`library`)
- deleting a recording others still hold takes its debrief along and keeps both as not on this PC until Download. `e2e` (`library`)
- deleting a recording nobody else holds removes it and its entry everywhere. `e2e` (`library`)
- a member cut off for a whole recording lists it as not on this PC and fetches it only when asked. `e2e` (`library`)
- A folder export opens in a video editor with every file lined up on the recording start. `manual`
- data: hours left come from the newest sessions. `unit`
- export: one input without delay is copied and the rest is filtered; the filter places each capture on the timeline; the encode arguments map only what exists; the folder export always starts every file at the recording start; events become notes chapters and subtitles on the recording clock; the export is sized and paced by its sources; file names are safe and distinct; exported file names never meet the files beside them; notes are written as csv; webvtt is written with numbered cues; finishing replaces the destination and leaves no part. `unit`
- log: append scan and repair a torn tail; extend takes whole records at the right offset only. `unit`
- main: the docs name every record kind and message the code writes; notes keep line breaks and tabs and drop other control and direction characters; json files may start with a byte order mark; whole writes replace the file and leave no part. `unit`
- room: a recording is removed completely only when no other member holds a part; deleting a recording removes only its folder and thumbnail; a recording removed completely takes its results and jobs with it; deleting a recording deletes only debriefs that name it and keeps peers copies. `unit`
- the delete preview lists every part that is here and says whether the recording goes completely; a row is on this PC or not, always shows a size, and deleting needs media here or nobody else; the two delete confirmations read the same in both languages; an imported recording is named by its title and the others by their time; the folder export is a menu action of the page. `ui-test`

## Updates and migration

- C kept every file of an older version until Delete old files, which removes exactly those. `e2e` (`library`)
- The installer run over a running app closes it, installs and starts the new version with the same rooms and recordings. `manual`
- The in-app update installs the new version, also on a PC that was on the Beta channel, ends your part first and cancels a running export, and while recording the app comes back into the recording in a new part. `manual`
- The old repository address still reaches the update manifest and the installer download. `todo`
- The installer run over GGI Hanseikai 0.19.4 leaves one app with its shortcuts, moves the data folder with the same ID (also when an empty new data folder exists), removes the old install folder when only ffmpeg.exe is left in it, and a 0.19.4 peer still connects. `manual`
- data: one app holds a data folder and a leftover lock is noticed; the data folder moves to its new name once and never while the old app runs, and when both names exist the old folder stays in use and the new one is untouched, unless the new one holds no key.bin and no rooms, which is replaced by the old one; a reset keeps the logs, and the rooms only when asked; a restart removes nothing under recordings and never touches old files; old files are exactly the older shapes; counts come from the files present with their logged size. `unit`
- main: settings from older versions fall back to defaults and keep only https relay URLs; rooms from older versions keep what still reads. `unit`

## API

- the local API is on from launch, reads recordings and takes a note and a game plan that everyone sees, and the page switches it off and on. `e2e` (`library`)
- api: only the loopback address and port are a valid host; the token must match exactly; requests are read with their query and body and paths decode strictly; malformed and oversized requests are refused; a wrong host or token gets nothing and llms txt needs no token; rooms list their members and skip pending ones; a recording whose open file belongs to an absent member reads as ended; recordings come newest first with members length and kind; events have times from the recording start tags and authors; chapters are the matches between match ends; the transcript lists spoken lines in time order with the speaker; the board is the newest team card and can be replaced; a note added through the api is an ordinary event with its tags; bad events are refused without writing; an imported recording shows its title importer and views; an operation is called by name with its json arguments; unknown paths and methods are refused; plugins show their runner and what is left and recordings their analysis; a debrief names its recording carries the watch and serves its voices; the app info names the open room; status tells the state of this pc and every member; diagnostics give this pcs report or everyones; every reference entry has lines and the main endpoints are listed; the address file holds the address and token for this user only; llms txt describes every endpoint. `unit`
- Deleting the AI notes, retrying a job or exporting a recording of a room that is not in the list is refused as not on this PC. `todo`
- ops: the page calls only listed operations and every operation is documented; arguments are read by name; displays get their size and windows do not. `unit`
- the AI menu offers a few prompts, or the switch when the API is off; a prompt names the API address, the token, llms.txt, the room and the recording; endpoint examples that are JSON are shown as indented code and the rest as text. `ui-test`
- The task of an AI prompt can be edited in the prompt dialog and is kept per purpose; Copy uses the edited task with the fixed address, token, room and recording lines; "Reset to default" restores the default and forgets the edit. `ui-test` (saved task, cleared task)

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
- the menu bar, with the line under it, is the same near-black as the title bar and the app on a fresh start, after a language or shortcut change (the menu is rebuilt) and after full screen (also on a PC set to the light theme), its drop-down menus are dark, and the menu bar is hidden while the player or the board is full screen and back after. `manual`
- three members record for an hour with Live open: the app's private memory and CPU stay flat after the first minutes and the heap keeps no growing free space (Windows segment heap). `manual`
- an hour-long recording is counted, checked for missing pieces and sized with one listing per folder, not work per segment, so a long session does not slow the app or grow its memory. `unit`
- check: a line starts with its level. `unit`
- debug: the transfer list keeps the newest; kinds follow live series and init; a piece after a failure is a retry; a hash failure marks the last receipt or adds a row; jobs are grouped with their latest state; reports hide addresses ids and names; reports hide a user folder with spaces; a job reads as one line; addresses are hidden and the rest kept; errors keep the newest warnings; the tail starts at a whole line; the flow text names each route and hides hosts; a combined report names each member by short id. `unit`
- hw: hardware is read from the script output; bad output gives empty hardware; the encoder names its kind and the matching gpu; the report line names the hardware encoder and capture rate; a healthy recording logs nothing until it ends; an idle screen is not a collapse; a stalled encoder is logged once until it recovers; short or voice only recordings end quietly; slow sends are logged rarely. `unit`
- shell: colors are written blue first for windows; translations are unique and fall back to english; the start command says what it records; only plain web links open; the hotkey needs ctrl or alt; closing asks only while work would be lost; clocks read like the player. `unit`
- sources: a closed window is never replaced by another window of the same program. `unit`
- days read as today, yesterday or a date; shortcuts need Ctrl or Alt and read like the native menu; the pages call a recording a recording, never a session; every English phrase and fixed message has a Japanese one and every Japanese entry is used; every end-to-end step is named in docs/test-cases.md. `ui-test`
- start page: an update or video-tools notice shows before any room is open; the two main actions read Join a room and Create a room. `manual` (UI walkthrough harness)
