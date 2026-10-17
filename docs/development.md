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

Releases do not contain ffmpeg: the app downloads a pinned build into its data folder on first run and checks its SHA-256. Pushing a tag such as `v0.1.1` (the same as the version in `src-tauri/tauri.conf.json`) runs `.github/workflows/release.yml`, which builds on Windows, signs the update with the key in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, publishes a pre-release with the installer and `latest.json`, and copies `latest.json` to the `updater` release that the app reads. The app checks for an update at start and installs it on request. Keep the private key out of the repository; only the public key is in `tauri.conf.json`.

To run a second instance on one PC for testing, set `EZH_DIR` to a folder inside the app data folder (it holds that instance's identity key, recordings and log) and, for a separate WebView2 profile, `WEBVIEW2_USER_DATA_FOLDER`.

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds, and starts the app. `APP_ARGS="--peer <id> --record 20 --audio-pid <pid>"` connects to a peer and records for 20 seconds. Set `SCRIPT=capture.ps1`, `audio.ps1` or `record.ps1` to also run a screen capture, a per-process audio capture or a combined recording check. `record-run.ps1 -TargetPid <pid>` records a running app instead of the test tone.
