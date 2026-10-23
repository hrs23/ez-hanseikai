![EZ Hanseikai](docs/logo.svg)

# EZ Hanseikai

Record your team's games and review them together.

A Windows app for recording and reviewing team games together. Each player records their own screen, game sound and voice; recordings and notes reach the others while they are still recording, so the team can review the last match while the next one records.

Any recording exports as a folder of plain files for a video editor.

Work in progress. See [the design](docs/design.md).

## Key principles

1. Made for team reviews: a squad watches its own matches together from everyone's view, not for clips or social sharing.
2. Everyone sees the same thing: while watching together, playback, layout, notes and analysis results match for all.
3. No busywork: everyone's recordings are collected and lined up automatically, with no manual syncing.
4. For people who trust each other: room members are trusted, and outsiders are kept out (encrypted connections, members only, one-time invites); the app does not try to defend a room against its own members.
5. No one is required: everyone holds everything, so the room keeps working when anyone leaves or is offline.
6. Any game: the app uses only the picture and sound, so it works with any game; game-specific features come from extensions.
7. Simple: as few settings, screens and features as the job needs.
8. Never touches the game: it only captures the screen and the game's sound, never injects code, draws overlays or reads game memory.
9. No cloud: recordings and notes go straight to your teammates and stay on your PCs.
10. Nothing is lost by one person's delete: everyone keeps their own copy.
11. Analysis is done by extensions that only read the recordings; each can be turned on or off on its own.
12. Free and open source.

## Install

1. Download `EZ.Hanseikai_<version>_x64-setup.exe` from [Releases](https://github.com/hrs23/ez-hanseikai/releases) and run it. It is unsigned, so SmartScreen warns: More info, then Run anyway. It installs for your Windows user only and needs no admin rights.
2. Start the app. Allow it through the firewall when Windows asks. On first run it downloads ffmpeg, and keeps retrying until that works.
3. Enter your name. One player presses Create room, which copies an invite code to send; the others paste it and press Join, and a member approves them (Copy invite code in the room's menu, the name at the top left, makes more). Choose your screen with the Screen button at the bottom left, then press Start recording.

Requires Windows 10 version 2004 or later, or Windows 11.

## Update

The app checks for a new version at start, every four hours while it runs (or Help > Check for updates), and shows it above the page with Install. Install works while recording or exporting too: it ends your part and cancels the export first, and afterwards the app is back in the room and, while the recording still runs, records again in a new part. Updates are signed and checked before they install. An older version may not read data from a newer one, so going back means starting with an empty data folder. Updating from GGI Hanseikai moves the data folder to [its name](#uninstall), removes the old app and its shortcuts, and Windows asks about the firewall again.

## Uninstall

Settings > Apps > Installed apps > EZ Hanseikai > Uninstall. Your data stays unless you tick the option to delete it, so a reinstall keeps your ID and recordings.

Data lives in `%LOCALAPPDATA%\dev.ez-hanseikai.app`; File > Open data folder opens it, and Help > Reset app resets it, keeping the log and, unless you choose otherwise, your ID, rooms and recordings. `key.bin` is your identity: keep it private, and deleting it changes your ID. To move your ID to another PC, copy `key.bin` while the app is closed.

## Security

- Only the teammates you connect to receive your recording, game sound, notes and name. Anything visible on your main screen, such as chats or notifications, is recorded too.
- Your ID is the public half of a key made on first start. Connections are encrypted and accepted only from room members. When two members reach each other only through a relay server, only voice, notes and logs go that way and the picture waits for a direct connection, so until then a member reachable only through a relay keeps the only copy of their picture. Settings > General lists relay servers to try before the public ones ([running one](docs/relay.md)); the one in use is published with your ID. An invite code works once; anyone who joins can invite others, so share it only with your team.
- The local API (Settings > API) is off by default, listens on 127.0.0.1 only (from another PC, forward the port over SSH), and needs a token, and then does what the window does: see [docs/llms.txt](docs/llms.txt).
- With the Screen button off, nothing of your screen is sent; what was already recorded still reaches the room. The upload limit (Settings > Capture) caps your upload. The app asks time.cloudflare.com for the time at start and every hour and sends nothing else there. Help > Diagnostics > Where data goes shows each connection and what goes over it.

## Contributing

1. Never touch the game process: capture the screen with Windows Graphics Capture and game sound with per-process audio capture; never inject code, draw overlays or read game memory; detect game events only from the captured picture and sound.
2. Everyday review features live in the app; game-specific analysis lives in extensions.
3. Extensions are self-contained: each runs as its own program, reads only recordings and set information, writes only events, depends on no other extension and can be turned on or off on its own. The app knows nothing about an extension's internals and shows every extension's events the same way.
4. One mechanism per job: notes, markers and extension results are all events. In the UI each action and piece of information has one place, with no explanatory text.
5. The UI is intuitive and standard, like common native apps: standard menus, shortcuts and dialogs, and everything works with the mouse alone.
6. The code stays plain: functions and modules, no clever abstractions, shared code only where it is truly the same, no comments.
7. Nothing is lost by a crash, a dropped connection or someone's delete, and everything resumes on its own.
8. Efficiency and simplicity come before the lowest delay; a few seconds of lag is fine.
9. No hard-coded numbers without a reason; keep the needed ones in one place.
10. Logic comes with automated tests, kept in plain functions that run without the network, capture or UI.
11. The repository is public: English only, made-up examples, and no secrets, personal data, real recordings or private addresses.
12. While in early development, breaking changes are fine and no migration code is added: the latest version only has to work with its own data. It must still start and work when files from older versions are present, ignoring what it does not understand and never deleting it. Clean up in Settings removes only files the app knows it made, moving all but temporary ones to the Recycle Bin, and never a file with an unknown name.
13. Add a script only when no existing mechanism does the job; fix the mechanism instead.

## Development

See [docs/development.md](docs/development.md). The docs are [the design](docs/design.md), [the storage format](docs/session-format.md), [the protocol](docs/protocol.md), [the local API](docs/llms.txt) and [development](docs/development.md).

## License

MIT or Apache-2.0, at your option.

The bundled fonts (Inter, Noto Sans JP, M PLUS 1) are under the SIL Open Font License 1.1, see `ui/fonts`.

The avatar and room crest shapes are from [game-icons.net](https://game-icons.net) by Lorc, Delapouite, DarkZaitzev, Carl Olsen, Caro Asercion and Skoll, under CC BY 3.0, see `ui/game-icons-LICENSE.txt`.
