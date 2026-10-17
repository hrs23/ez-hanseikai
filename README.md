# EZ Hanseikai

Team reviews made ez.

A desktop app for recording and reviewing team games together. Everyone installs the same app, joins a room, and records their own screen, game sound and voice. Recordings and notes sync between the members, so the team can watch the last match together while the next one is still recording.

Work in progress: only an empty window opens so far. See [the design](docs/design.md).

## Run

On Windows with Node.js, Rust (MSVC) and the Visual Studio C++ Build Tools:

```
npm install
npm run dev
```

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds and starts the app. Set `SCRIPT=capture.ps1`, `audio.ps1` or `record.ps1` to also run a screen capture, a per-process audio capture or a combined recording check. `record-run.ps1 -TargetPid <pid>` records a running app instead of the test tone.

## License

MIT or Apache-2.0, at your option.
