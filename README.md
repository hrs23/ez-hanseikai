# EZ Hanseikai

Team reviews made ez.

A desktop app for recording and reviewing team games together. Everyone installs the same app, joins a room, and records their own screen, game sound and voice. Recordings and notes sync between the members, so the team can watch the last match together while the next one is still recording.

Work in progress. See [the design](docs/design.md).

## Use

Install the setup exe, start the app, copy your ID to the other person, paste theirs and press Connect, then Record. Windows asks once to allow the app through the firewall; allow it. The setup is unsigned, so SmartScreen warns: More info, then Run anyway.

The app records the primary monitor and, if you pick a process in "Game sound from", that process's sound. It only captures; it never touches the game. Recordings and logs are in `%APPDATA%\dev.ez-hanseikai.app`. Network details and "Save report" are under Network.

## Build

On Windows with Node.js, Rust (MSVC), the Visual Studio C++ Build Tools and an ffmpeg with NVENC, AMF, QSV and libx264 in PATH:

```
npm install
copy ffmpeg to src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe
npm run build
```

The setup exe is in `src-tauri/target/release/bundle/nsis`. It bundles that ffmpeg, so check its licence before sharing the installer.

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds, and starts the app. `APP_ARGS="--peer <id> --record 20 --audio-pid <pid>"` connects to a peer and records for 20 seconds. Set `SCRIPT=capture.ps1`, `audio.ps1` or `record.ps1` to also run a screen capture, a per-process audio capture or a combined recording check. `record-run.ps1 -TargetPid <pid>` records a running app instead of the test tone.

## License

MIT or Apache-2.0, at your option.
