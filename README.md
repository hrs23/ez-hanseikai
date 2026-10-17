# EZ Hanseikai

Team reviews made ez.

A desktop app for recording and reviewing team games together. Everyone installs the same app, joins a room, and records their own screen, game sound and voice. Recordings and notes sync between the members, so the team can watch the last match together while the next one is still recording.

Work in progress. See [the design](docs/design.md).

## Use

Install the setup exe, start the app, copy your ID to the others, paste each of theirs and press Connect (everyone adds everyone else), then Record. Windows asks once to allow the app through the firewall; allow it. The setup is unsigned, so SmartScreen warns: More info, then Run anyway.

The app records the primary monitor and, if you pick a process in "Game sound from", that process's sound. It only captures; it never touches the game. Recordings and logs are in `%APPDATA%\dev.ez-hanseikai.app`. Network details and "Save report" are under Network.

## Security

- What is shared: your recording of the whole main screen (and the chosen game's sound), your notes and your display name go to every peer you add, and only to them. Notifications or chats visible on screen are recorded too. Nothing goes to a server; recordings stay on the players' PCs.
- IDs: your ID is the public half of a key pair that the app makes on first start. Only peers whose IDs you added can connect, and each connection is encrypted and tied to that key. Sharing your ID alone gives nobody access; the other person must add yours too. Removing a peer closes its connection at once.
- The installer and the app are unsigned, so Windows SmartScreen warns. Updates are signed and checked before they are installed. The app never touches the game: it only captures the screen and sound.

## Where files are stored and how to uninstall

Everything the app writes is in `%LOCALAPPDATA%\dev.ez-hanseikai.app` (older versions used `%APPDATA%`; the app moves that data on first start). The Files section of the app shows the folder, the space used and the free space, and opens the folder.

- `key.bin`: your identity key. Keep it private; it is never sent anywhere. Deleting it changes your ID.
- `peers.txt`, `settings.json`: the peer list and your display name.
- `log.bin` and `rec-*.mp4`: your event log and recordings. `peer\`: what other players sent you.
- `ffmpeg.exe`: the video tool, downloaded on first run.
- `log.txt`, `report.txt`, `id.txt`, `version.txt`: logs and small status files.

To uninstall, use Settings > Apps > Installed apps, or `uninstall.exe` in `%LOCALAPPDATA%\EZ Hanseikai`. The uninstaller removes the app, its Start menu entry and its shortcut and closes a running copy. Your data stays by default, so reinstalling keeps your ID; to remove it too, tick the uninstaller's option to delete the application data (this also deletes the identity key and recordings), or use Reset in the app's Files section first.

Files > Reset has two levels: settings and cache (keeps recordings and your ID), and everything (needs typing RESET; recordings and the ID cannot be recovered). If the previous start did not finish, or the key cannot be read, the app shows a recovery screen with the folder, a report and a reset instead of failing; it never deletes or overwrites `key.bin` by itself.

To roll back after a bad update, uninstall without deleting the data, then install the previous version from GitHub Releases. File formats have not changed between 0.1.x versions, so older versions read newer data; a later format change would be noted in the release notes.

## Build

On Windows with Node.js, Rust (MSVC), the Visual Studio C++ Build Tools and an ffmpeg with NVENC, AMF, QSV and libx264 in PATH:

```
npm install
copy ffmpeg to src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe
npm run build
```

The setup exe is in `src-tauri/target/release/bundle/nsis`. `scripts/build-installer.ps1` builds an offline installer that bundles that ffmpeg (a GPL build, so do not publish it) and writes its SHA-256 file.

## Release

Releases do not contain ffmpeg: the app downloads a pinned build into its data folder on first run and checks its SHA-256. Pushing a tag such as `v0.1.1` (the same as the version in `src-tauri/tauri.conf.json`) runs `.github/workflows/release.yml`, which builds on Windows, signs the update with the key in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, publishes a pre-release with the installer and `latest.json`, and copies `latest.json` to the `updater` release that the app reads. The app checks for an update at start and installs it on request. Keep the private key out of the repository; only the public key is in `tauri.conf.json`.

To run a second instance on one PC for testing, set `EZH_DIR` to a folder inside the app data folder (it holds that instance's identity key, recordings and log) and, for a separate WebView2 profile, `WEBVIEW2_USER_DATA_FOLDER`.

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds, and starts the app. `APP_ARGS="--peer <id> --record 20 --audio-pid <pid>"` connects to a peer and records for 20 seconds. Set `SCRIPT=capture.ps1`, `audio.ps1` or `record.ps1` to also run a screen capture, a per-process audio capture or a combined recording check. `record-run.ps1 -TargetPid <pid>` records a running app instead of the test tone.

## License

MIT or Apache-2.0, at your option.
