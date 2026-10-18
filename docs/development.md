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

Releases do not contain ffmpeg: the app downloads a pinned build into its data folder on first run and checks its SHA-256. To release, raise the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json` and push to main. When the tests pass on main and no tag for that version exists, `.github/workflows/release.yml` tags the commit, builds on Windows, signs the update with the key in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, publishes a pre-release with the installer and `latest.json`, and copies `latest.json` to the `updater` release that the app reads. The app checks for an update at start and installs it on request. Keep the private key out of the repository; only the public key is in `tauri.conf.json`.

To run a second instance on one PC for testing, set `EZH_DIR` to a folder inside the app data folder (it holds that instance's identity key, recordings and log) and, for a separate WebView2 profile, `WEBVIEW2_USER_DATA_FOLDER`.

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds, and starts the app.

A deploy builds without an installer (`tauri build --no-bundle`); set `INSTALLER=1` to also build the installer, or `TEST=1` to only run the unit tests (`cargo test` in `src-tauri`). GitHub Actions runs the same tests on every push.

Set `EZH_RELAY_ONLY=1` to start an instance that uses only the relay, to test the slow path.

Set `EZH_TEST=<port>` for test mode: capture encodes a generated picture and two tones (ffmpeg `testsrc2` and `sine`) instead of the screen, app sound and microphone, Export set writes into the data folder without a dialog, and the window's page accepts DevTools (CDP) connections on that local port.

The end-to-end test `node scripts/e2e.mjs <app exe> <folder with ffmpeg.exe and ffprobe.exe>` starts three test-mode instances on one PC, each with its own data folder, WebView2 profile and remote-debugging port, and drives them through their pages: joining with approval, a room-wide recording with a late joiner, live segments and history checked by SHA-256, a note with its edit and deletion, a crash and restart mid-recording that rejoins the set, an encoder killed mid-recording, a lost own log restored before new records, Watch Party, an Apex matches job, an export checked with ffprobe and removing a member. GitHub Actions runs it on every push after a release build and keeps the screenshots, states and logs when it fails.
