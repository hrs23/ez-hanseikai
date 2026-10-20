# Local API

A local HTTP API lets a person, an AI agent or another program read a room's recordings, write notes and the game plan, and import and export recordings as [session folders](session-format.md). It uses the same room-log events as the app, so other members see what is written. [llms.txt](llms.txt) is a short description of it for AI agents; the app serves the same text at `GET /llms.txt`.

## Turn it on

Off by default. Settings > API > Local API starts it. The page shows the address, `http://127.0.0.1:47625`, and Copy token puts the token on the clipboard; New token replaces it (programs using the old one stop working). Below them the page lists the endpoints with an example each, and opens `/llms.txt`. The switch, the token and the exposure are kept in `settings.json` (`api`, `api_token`, `api_lan`). An API badge in the room header shows while the API runs; it opens this page. The AI button in a room, and in the player, copies a ready prompt with the address, the token and a pointer to `/llms.txt`, or turns the API on.

Starting the app with `--api`, or with `EZH_API=1`, turns the API on at start (and keeps the switch on). `"api": true` in `settings.json` does the same.

While the API runs, the address and token are also written to `api.json` in the data folder, `%LOCALAPPDATA%\dev.ez-hanseikai.app\api.json`, readable by the current Windows user only: `{"address": "127.0.0.1:47625", "token": "<token>"}`. The file is removed when the API stops or the app exits, and rewritten with a new token. A program on this PC can read it instead of asking the person.

The port is 47625. If another program holds it, Settings says so. `EZH_API_PORT` sets another port, for tests.

## Other devices

Settings > API > Access has two choices. This PC only (the default) listens on `127.0.0.1`. Other devices listens on all interfaces, so another PC on the LAN or a Tailscale network can use the API at the PC's address, such as `http://192.0.2.20:47625`. Settings and the API badge warn that other devices on the network can control the app. The token is still needed, and the change applies at once.

The traffic is plain HTTP, not encrypted: use it on networks you trust only. Windows may ask to allow the app through its firewall.

## Rules

- It listens on `127.0.0.1` only, unless Other devices is on. A request whose `Host` header is not `127.0.0.1:<port>` or `localhost:<port>` gets 403, which stops DNS rebinding; with Other devices on, this PC's own IPv4 addresses and its computer name (with the port) are accepted too, and anything else still gets 403. There are no CORS headers.
- Every request except `GET /llms.txt` needs `Authorization: Bearer <token>`; otherwise 401. The token is 48 random hex characters.
- JSON in and out, UTF-8, one request per connection. Errors are `{"error": "..."}` with 400, 401, 403, 404, 405, 409, 413 or 503.
- Times inside a recording are seconds from its start, rounded to 0.1 (`at_s`, `from_s`, `to_s`). Clock times are milliseconds since 1970 (`start_ms`, `end_ms`, `updated_ms`).
- Reads work for every room on this PC. Writes work only for the room that is open in the app, because only that room is shared with the others; otherwise 409.
- Ids with spaces (a recording id looks like `2026-10-19 21-30-00`) are written percent-encoded in the path.
- Notes are at most 500 characters and the game plan at most 4000. Bodies are at most 64 kB.
- There are no endpoints for clips. A recording leaves as a folder of plain files (`export`), and comes in from video files or such a folder (`import`).
- Paths are on this PC. In JSON write `/` (`C:/videos/scrim`) or double the backslashes.

## Endpoints

### `GET /llms.txt`

The text of [llms.txt](llms.txt). No token.

### `GET /v1`

```json
{"app": "EZ Hanseikai", "version": "0.12.0", "id": "<own id>", "name": "Ann", "room": "ab12", "recording": false}
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
  "members": [{"id": "<id>", "name": "Ann", "voice_only": false, "imported": false, "start_ms": 1792445400000, "end_ms": 1792448100000}]
}]}
```

`kind` is `play`, or `debrief` for a recording of voices made while watching another one together. `end_ms` is `null` while recording. An imported recording has its `title`, `imported_by` (`{"id", "name"}` of the person who imported it) and `imported: true` on each member, which is a view with the name it was given.

### `GET /v1/rooms/{room}/recordings/{recording}`

One element of the list above.

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

Notes, marks and plugin results in time order, untimed ones first. `plugin` is the plugin's name for analysis results and `null` for people's events. `member` is the player a result is about. Without `kind`, spoken lines (`speech`), drawings and game plan cards are left out.

### `GET /v1/rooms/{room}/recordings/{recording}/chapters`

```json
{"chapters": [{"n": 1, "name": "Match 1", "from_s": 0.0, "to_s": 612.3}, {"n": 2, "name": "Match 2", "from_s": 612.3, "to_s": null}]}
```

The matches between the match ends that the Apex matches plugin found. `to_s` is `null` for the match in progress. Empty when the plugin is off.

### `GET /v1/rooms/{room}/recordings/{recording}/transcript`

Query: `member=<id>`.

```json
{"lines": [{"member": "<id>", "member_name": "Ren", "from_s": 5.0, "to_s": 8.0, "text": "Drop north"}]}
```

Spoken lines from the Transcription plugin in time order. Empty when it is off or not finished.

### `GET /v1/rooms/{room}/board`

```json
{"text": "# This week\n**Move before the ring closes**", "author": "<id>", "author_name": "Ann", "updated_ms": 1792445000000}
```

The game plan. `text` is `""` and the rest `null` when nobody has written one.

### `PUT /v1/rooms/{room}/board`

```json
{"text": "# This week\n**Move before the ring closes**\n\n## Next match\n- Pick the drop first"}
```

Replaces the game plan for the whole room and answers 200 with the new board (same shape as the read). `text` must not be empty.

### `POST /v1/rooms/{room}/recordings/{recording}/events`

```json
{"text": "Late rotate, the ring was closing", "at_s": 1325.5, "tags": ["mistake", "rotate"]}
```

`kind` is `note` (default) or `mark`. A note needs `text`; `at_s` is optional, and a note without it belongs to the whole recording. A mark needs `at_s` and has no text. `tags` are appended to the text as `#tag` unless it already has them. Answers 201 with the event, shaped as in the list.

### `POST /v1/rooms/{room}/open`

No body. Opens the room in the app, so that writes go to it, and answers 200 `{"room": "ab12"}`. 409 while this PC records. Starting and stopping a recording is not in the API: it needs the person's confirmation in the app.

### `POST /v1/rooms/{room}/recordings/import`

```json
{"folder": "C:/videos/Scrim night", "title": "Scrim night", "names": ["Ann", "Ben"]}
```

or `{"files": ["C:/videos/ann.mp4", "C:/videos/ben.mp4"]}`. Makes a recording from a [session folder](session-format.md) or from video files, shares it with the room like a recorded one, and answers 201 when it is done (as long as the copying takes):

```json
{"id": "2026-10-19 21-30-00", "views": 2, "notes": 3}
```

`title` and `names` (one name per view, in order) are optional and replace the folder's own. 400 says what is wrong with the files (nothing is imported then), 409 means the room is not the open one or another import is running.

### `POST /v1/rooms/{room}/recordings/{recording}/export`

```json
{"folder": "C:/exports/scrim", "aligned": false}
```

Writes the recording into a new or empty folder on this PC as a [session folder](session-format.md), and answers 201 when it is done:

```json
{"folder": "C:/exports/scrim", "files": 2, "skipped": []}
```

`aligned` (default `false`) pads every file so all start together. `skipped` lists views that are not on this PC yet. 400 for a folder that is not empty, 409 while a recording or another export runs.

### `GET /v1/rooms/{room}/plugins`

```json
{"plugins": [{"id": "transcribe", "name": "Transcription", "version": "1.0.0", "on": true, "settings": {"model": "small"}, "ready": true, "process_here": true,
  "workers": [{"id": "<id>", "name": "Ren", "on": true, "ready": true}]}]}
```

`on` is the room's choice; `ready` means this PC has the plugin's files; `process_here` is this PC's switch; `workers` is who processes it.

### `GET /v1/rooms/{room}/jobs`

```json
{"jobs": [{"job": "transcribe@1.0.0/ab12cd/2026-10-19 21-30-00/<id>", "plugin": "transcribe", "version": "1.0.0", "set": "2026-10-19 21-30-00", "owner": "<id>", "owner_name": "Ren", "whole": false, "state": "waiting", "by": null, "error": null}],
 "running": {"job": "...", "plugin": "transcribe", "progress": 0.4}}
```

`state` is `waiting`, `claimed`, `done` or `failed`; `by` is the member who runs or ran it; `whole` marks a job for the whole recording rather than one member. `running` is the job this PC runs now, or `null`.

### `GET /v1/diagnostics`

```json
{"text": "ez-hanseikai 0.16.2 on windows ...\n\nrecent errors\n..."}
```

The plain-text report of Help > Diagnostics > Copy report, without its summary lines, with names and addresses hidden. `?everyone=true` (or `1`) adds each connected member's report under `=== member <id> ===`, as they share it. It runs the environment check, so it can take a few seconds.

## Example

```
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:47625/v1/rooms
curl -H "Authorization: Bearer $TOKEN" -d '{"text":"Good call #callout","at_s":84}' \
  "http://127.0.0.1:47625/v1/rooms/ab12/recordings/2026-10-19%2021-30-00/events"
curl -H "Authorization: Bearer $TOKEN" -d '{"folder":"C:/videos/Scrim night"}' \
  http://127.0.0.1:47625/v1/rooms/ab12/recordings/import
```

## Code

`src-tauri/src/api.rs` holds the server, the checks (host, token, request size) and the handlers, which work on a small `Backend` trait so that tests run on plain data. `docs/llms.txt` is embedded in the app at build time.
