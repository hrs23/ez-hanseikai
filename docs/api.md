# Local API

A local HTTP API lets a person, an AI agent or another program read a room's recordings and do everything the app's window does, except open windows. Recordings come in and go out as [session folders](session-format.md). It uses the same room-log events as the app, so other members see what is written. [llms.txt](llms.txt) is a short description of it for AI agents; the app serves the same text at `GET /llms.txt`.

## Turn it on

Off by default. Settings > API > Local API starts it. The page shows the address, `http://127.0.0.1:47625`, and Copy token puts the token on the clipboard; New token replaces it (programs using the old one stop working). Below them the page lists the endpoints with an example each, and opens `/llms.txt`. The switch, the token and the exposure are kept in `settings.json` (`api`, `api_token`, `api_lan`). An API badge at the top right, beside the AI button, shows while the API runs; it opens this page. The AI button in a room, and in the player, copies a ready prompt with the address, the token and a pointer to `/llms.txt`, or turns the API on.

Starting the app with `--api`, or with `EZH_API=1`, turns the API on at start (and keeps the switch on). `"api": true` in `settings.json` does the same.

While the API runs, the address and token are also written to `api.json` in the data folder, `%LOCALAPPDATA%\dev.ggi-hanseikai.app\api.json`, readable by the current Windows user only: `{"address": "127.0.0.1:47625", "token": "<token>"}`. The file is removed when the API stops or the app exits, and rewritten with a new token. A program on this PC can read it instead of asking the person.

The port is 47625. If another program holds it, Settings says so. `EZH_API_PORT` sets another port, for tests.

## Other devices

Settings > API > Access has two choices. This PC only (the default) listens on `127.0.0.1`. Other devices listens on all interfaces, so another PC on the LAN or a Tailscale network can use the API at the PC's address, such as `http://192.0.2.20:47625`. Settings and the API badge warn that other devices on the network can control the app. The token is still needed, and the change applies at once.

The traffic is plain HTTP, not encrypted: use it on networks you trust only. Windows may ask to allow the app through its firewall.

## Rules

- It listens on `127.0.0.1` only, unless Other devices is on. A request whose `Host` header is not `127.0.0.1:<port>` or `localhost:<port>` gets 403, which stops DNS rebinding; with Other devices on, this PC's own IPv4 addresses and its computer name (with the port) are accepted too, and anything else still gets 403. There are no CORS headers.
- Every request except `GET /llms.txt` needs `Authorization: Bearer <token>`; otherwise 401. The token is 48 random hex characters.
- JSON in and out, UTF-8, one request per connection. Errors are `{"error": "..."}` with 400 (also for an operation the app refuses, with the message the window would show), 401, 403, 404 (also for an unknown operation), 405, 409, 413, 500 or 503.
- Times inside a recording are seconds from its start, rounded to 0.1 (`at_s`, `from_s`, `to_s`). Clock times are milliseconds since 1970 (`start_ms`, `end_ms`, `updated_ms`).
- Reads work for every room on this PC. Operations act on the open room as the window does, and events and the board are written only to the open room, because only that room is shared with the others; otherwise 409.
- Operations run at once, without the window's confirmations (starting and stopping a recording, deleting, removing a room, resetting). `delete_set`, `remove_room`, `delete_old_files` and `reset` with `keep_rooms: false` remove recordings or the app's data with no undo: call them only when the person asked for exactly that. The token gives the same control as the window, also from the LAN with Other devices.
- Ids with spaces (a recording id looks like `2026-10-19 21-30-00`) are written percent-encoded in the path.
- Notes are at most 500 characters and the game plan at most 4000. Bodies are at most 64 kB.
- There are no endpoints for clips. A recording leaves as a folder of plain files (`export_folder {room, set, folder}`); see [the storage format](session-format.md#export-folder).
- Paths are on this PC. In JSON write `/` (`C:/videos/scrim`) or double the backslashes.

## Endpoints

### `GET /llms.txt`

The text of [llms.txt](llms.txt). No token.

### `GET /v1`

```json
{"app": "GGI Hanseikai", "version": "0.12.0", "id": "<own id>", "name": "Ann", "room": "ab12", "recording": false}
```

`room` is the open room (or `null`); `recording` is true while a recording runs.

### `GET /v1/status`

```json
{"room": "ab12", "recording": false, "state": "standby", "members": [{"id": "<id>", "name": "Ann", "online": true, "state": "standby"}]}
```

`state` is `offline`, `idle` (no screen shared), `standby` (sharing a screen) or `recording`. The top-level `state` is this PC's; `members` are those of the open room.

### `GET /v1/rooms`

```json
{"rooms": [{"id": "ab12", "name": "Practice", "active": true, "members": [{"id": "<id>", "name": "Ann", "online": true}]}]}
```

### `GET /v1/rooms/{room}/recordings`

Newest first.

```json
{"recordings": [{
  "id": "2026-10-19 21-30-00", "kind": "play", "recording": false,
  "start_ms": 1792445400000, "end_ms": 1792448100000, "length_s": 2700.0,
  "title": null, "imported_by": null,
  "members": [{"id": "<id>", "name": "Ann", "voice_only": false, "imported": false, "start_ms": 1792445400000, "end_ms": 1792448100000}],
  "analysis": [{"plugin": "transcribe", "state": "running", "progress": 0.4}]
}]}
```

`kind` is `play`, or `debrief` for a recording of voices made while watching another one together. A debrief also has `of`, the id of the recording it is about, and `watch`, what the Watch Party showed while it was recorded: `[{"t_us", "by", "set", "at_us", "playing", "rate"}]` in microseconds on this PC's clock. `end_ms` is `null` while recording. `analysis` has one entry per extension that is on, for the recording: `state` is `done`, `running` (with `progress` on the runner's PC), `left` (with `reason`: `queue`, `receiving`, `waiting` with `waiting_for`, the runner while its PC is offline, or `no_runner`) or `failed` (with `error`, or `reason` `missing` when no member online holds the file). A recording placed from outside (see [the storage format](session-format.md#placing-a-recording-made-elsewhere)) has its `title`, `imported_by` (`{"id", "name"}` of the member who placed it) and `imported: true` on each member, which is a view with the name it was given.

### `GET /v1/rooms/{room}/recordings/{recording}`

One element of the list above.

### `GET /v1/rooms/{room}/recordings/{recording}/voices/{member}`

The member's voice in the recording as raw fragmented MP4 (`audio/mp4`): each capture's `init.mp4` and segments, and when the member has several captures (the app restarted while recording) they follow one another in time order, each starting with its own `ftyp`. 404 when the member has no voice in the recording. A debrief's voices are read here, so an agent can transcribe them.

### `GET /v1/rooms/{room}/recordings/{recording}/events`

Query: `tag=<tag>` (case-insensitive) and `kind=<kind>`.

```json
{"events": [{
  "key": "<author id>/1792445520000000", "id": 1792445520000000,
  "author": "<id>", "author_name": "Ann", "plugin": null, "kind": "note",
  "at_s": 120.5, "to_s": null, "member": null, "member_name": null,
  "text": "Rotate early #rotate", "tags": ["rotate"], "values": null
}]}
```

Notes, marks and extension results in time order, untimed ones first. A note or mark written through this API has `source: "ai"` in the log; the app marks it with AI and the recording menu's Delete AI notes hides them. `plugin` is the plugin's name for analysis results and `null` for people's events. `member` is the player a result is about. Without `kind`, spoken lines (`speech`), drawings and game plan cards are left out.

### `GET /v1/rooms/{room}/recordings/{recording}/chapters`

```json
{"chapters": [{"n": 1, "name": "Match 1", "from_s": 0.0, "to_s": 612.3}, {"n": 2, "name": "Match 2", "from_s": 612.3, "to_s": null}]}
```

The matches between the match ends that the Apex matches extension found. `to_s` is `null` for the match in progress. Empty when the extension is off.

### `GET /v1/rooms/{room}/recordings/{recording}/transcript`

Query: `member=<id>`.

```json
{"lines": [{"member": "<id>", "member_name": "Ren", "from_s": 5.0, "to_s": 8.0, "text": "Drop north"}]}
```

Spoken lines from the Transcription extension in time order. Empty when it is off or not finished.

### `GET /v1/rooms/{room}/board`

```json
{"text": "**Move before the ring closes**\n- Pick the drop first", "author": "<id>", "author_name": "Ann", "updated_ms": 1792445000000}
```

The team card of the game plan. `text` is `""` and the rest `null` when nobody has written one. The other cards are written with the `put_board_card` operation.

### `PUT /v1/rooms/{room}/board`

```json
{"text": "**Move before the ring closes**\n- Pick the drop first"}
```

Replaces the team card for the whole room and answers 200 with the new board (same shape as the read). `text` must not be empty.

### `POST /v1/rooms/{room}/recordings/{recording}/events`

```json
{"text": "Late rotate, the ring was closing", "at_s": 1325.5, "tags": ["mistake", "rotate"]}
```

`kind` is `note` (default) or `mark`. A note needs `text`; `at_s` is optional, and a note without it belongs to the whole recording. A mark needs `at_s` and has no text. `tags` are appended to the text as `#tag` unless it already has them. Answers 201 with the event, shaped as in the list.

### `POST /v1/ops/{name}`

The body is a JSON object with the operation's arguments (empty means `{}`). Answers 200 with the operation's value exactly as the window gets it (`null` for most), 404 `No such operation.` for a name that does not exist, 400 `{"error": "..."}` when the app refuses (the message the window would show). `src-tauri/src/ops.rs` is the one list: the window and this route call the same functions, so whatever the window can do is here.

Long operations hold the connection until they are done: `export_folder`, `prepare_plugin`, `check_peer` (up to 12 s for this PC), `check_update`, `debug_report`. `reset` and `install_update` restart the app, so their request ends without an answer. After `set_api` with `on: false` the API is off, after `new_api_token` the next request needs the new token (read `api.json`), and `set_api_lan` restarts the listener on the new address. Arguments marked `?` are optional; the rest are required (`<name> is missing or not valid.`). Ids and values are those of `GET` answers and of the window's state.

The operations, with their arguments and answers, are listed in [llms.txt](llms.txt) (also under Settings > API).

Not available: the operations that open a window, that is choosing files or folders, opening links and folders, and the identity file.

### `GET /v1/rooms/{room}/plugins`

```json
{"plugins": [{"id": "transcribe", "name": "Transcription", "version": "1.0.0", "on": true, "settings": {"model": "small"}, "ready": true,
  "runner": "<id>", "runner_name": "Ren", "left": 2, "failed": 0, "waiting_for": null, "waiting_for_name": null}]}
```

`on` is the room's choice; `ready` means this PC has the extension's files. `runner` is the one member whose PC processes every recording for it (`null` when that member left the room); `left` counts recordings still to do and `failed` those that failed, and `waiting_for` is the runner while its PC is offline. `retry_job` asks for a recording to be analyzed again.

### `GET /v1/diagnostics`

```json
{"text": "ggi-hanseikai 0.16.2 on windows ...\n\nrecent errors\n..."}
```

The plain-text report of Help > Diagnostics > Copy report, without its summary lines, with names and addresses hidden. `?everyone=true` (or `1`) puts this PC's report under `=== this PC ===` and adds each connected member's report, as they share it, under `=== member <first 10 characters of the id> ===`. It runs the environment check, so it can take a few seconds.

## Example

```
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:47625/v1/rooms
curl -H "Authorization: Bearer $TOKEN" -d '{"text":"Good call #callout","at_s":84}' \
  "http://127.0.0.1:47625/v1/rooms/ab12/recordings/2026-10-19%2021-30-00/events"
curl -H "Authorization: Bearer $TOKEN" -d '{"room":"ab12","set":"2026-10-19 21-30-00","folder":"C:/videos/Scrim night"}' \
  http://127.0.0.1:47625/v1/ops/export_folder
```

## Code

`src-tauri/src/api.rs` holds the server, the checks (host, token, request size) and the handlers, which work on a small `Backend` trait so that tests run on plain data. `src-tauri/src/ops.rs` holds the one list of operations that the window's `call` command and the API's `/v1/ops` route dispatch into. `docs/llms.txt` is embedded in the app at build time.
