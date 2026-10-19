# Development

## Build

On Windows with Node.js, Rust (MSVC), the Visual Studio C++ Build Tools and an ffmpeg with NVENC, AMF, QSV and libx264 in PATH:

```
npm install
copy ffmpeg to src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe
npm run build
```

The setup exe is in `src-tauri/target/release/bundle/nsis`. `scripts/build-installer.ps1` builds an offline installer that bundles that ffmpeg (a GPL build, so do not publish it) and writes its SHA-256 file.

## Release

Releases do not contain ffmpeg: the app downloads a pinned build into its data folder on first run and checks its SHA-256. To release, raise the version in `src-tauri/Cargo.toml` (`Cargo.lock` follows on the next build) and push to main. The end-to-end job builds the installer, signed with the key in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (unsigned, and not released, without them). When the tests and the end-to-end test pass on main and no release for that version exists, the `release` job in `.github/workflows/test.yml` publishes that tested installer with `latest.json`. Every run on main then copies that release's manifest to the `updater` release that the app reads: always as `latest-beta.json` (the Beta channel), and as `latest.json` (the Stable channel) only when the version has no pre-release part. So `0.9.1-1` reaches Beta only, and `0.9.1` reaches both. Use a numeric pre-release part, which every Windows installer format accepts. Keep the private key out of the repository; only the public key is in `tauri.conf.json`.

To run a second instance on one PC for testing, set `EZH_DIR` to a folder inside the app data folder (it holds that instance's identity key, recordings and log) and, for a separate WebView2 profile, `WEBVIEW2_USER_DATA_FOLDER`.

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds, and starts the app.

A deploy builds without an installer (`tauri build --no-bundle`); set `INSTALLER=1` to also build the installer, or `TEST=1` to only run the unit tests (`cargo test` in `src-tauri`). GitHub Actions runs the same tests on every push.

Start the app with `--api` or set `EZH_API=1` to turn the local API on at start. Set `EZH_API_PORT=<port>` to move the local API off its default port 47625 (the end-to-end test gives each instance its own). The API's unit tests are in `src-tauri/src/api.rs`; `docs/llms.txt` is embedded in the app and drives the endpoint list in Settings (a test checks it against `docs/api.md`), so edit them together.

The hardware and clock code is in `src-tauri/src/hw.rs`; its tests feed fake PowerShell, `sc` and `w32tm` output, and `scripts/ui-test.mjs` covers the PCs and Where data goes rows with made-up data.

The import and the folder export need ffmpeg: the app uses the one it downloaded, and the end-to-end test makes its tiny test videos with the ffmpeg in its folder. `docs/session-format.md` is the format; `src-tauri/src/session.rs` holds its parsers and their tests.

Set `EZH_RELAY_ONLY=1` to start an instance that uses only the relay, to test the slow path.

Set `EZH_TEST=<port>` for test mode: capture encodes a generated picture and two tones (ffmpeg `testsrc2` and `sine`) instead of the screen, app sound and microphone, Export set writes into the data folder without a dialog, and the window's page accepts DevTools (CDP) connections on that local port.

The end-to-end test `node scripts/e2e.mjs <app exe> <folder with ffmpeg.exe and ffprobe.exe>` starts three test-mode instances on one PC, each with its own data folder, WebView2 profile and remote-debugging port, and drives them through their pages after choosing a capture source: a recording made alone whose player follows the growing picture, joining with approval on one connection, a second app on the same data folder that exits, connections that stay up without redialing, a room-wide recording with a late joiner, live segments and history checked by SHA-256, a note with its edit and deletion, the confirmation before recording starts or stops and the notice the others get, a crash and restart mid-recording that recovers without a message and rejoins the set, an encoder killed mid-recording, a lost own log restored before new records, an Apex matches job run once for the set, the transcription plugin following the chosen model with one job row per person, one plugin's processing switched off by a member and seen by all, the Who runs it choice (a chosen member alone runs a job, and the only member who processes is named for Auto), Live showing a tile for everyone who shares a screen (idle and while recording, marked apart from the running recording) and being where the room opens, the room menu and a rename from Room settings seen by all, the game plan rendering Markdown (tables, code, tasks, links, and unsafe links and tags left as text), ticking two tasks at once, and a personal game plan that stays on its PC, the sidebar's width (limits, no sideways scroll with a long room name, dragging the splitter, kept, clear of the list's scrollbar), thumbnails of ended and imported recordings, every recording in the sidebar and the Recordings view with its filter, one short state per member, dragging a name label to swap two tiles, a Diagnostics screen whose tabs open and whose report holds the hardware, clocks and where-data-goes sections and hides names, IDs, addresses and paths, the record menu with Record debrief, the local API (off at first, then switched on in the page: api.json with the address and token, llms.txt without a token, a wrong Host or token refused, the recordings read, the status, everyone's diagnostics report without names and opening the room, the API badge and the endpoint list in Settings, Other devices reached by the PC's own address, a note and a game plan written through it and seen in the UI and by the others, switched off again), Watch Party, a debrief recorded while watching together (voice only, with `party` records) and replayed, an export checked with ffprobe, a folder export (raw and aligned) read with ffprobe and imported back as a recording that every member receives by SHA-256 and plays, a hand-made folder with a converted file and a sound-only file imported on another member, the API importing a bare folder and exporting it, finished and imported recordings fetched only when Download is pressed, deleting a recording others still hold (it stays as not on this PC and downloads again), deleting one nobody else holds (its entry and job rows disappear on the others) and discarding a short recording, the update channel setting, removing a member, and leaving and entering the room again. GitHub Actions runs it on every push after a release build and keeps the screenshots, states and logs when it fails.
