# Development

## Build

On Windows with Node.js, Rust (MSVC), the Visual Studio C++ Build Tools and an ffmpeg with NVENC, AMF, QSV and libx264 in PATH:

```
npm install
copy ffmpeg to src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe
npm run build
```

The setup exe is in `src-tauri/target/release/bundle/nsis`. `scripts/build-installer.ps1` builds an offline installer that bundles that ffmpeg (a GPL build, so do not publish it) and writes its SHA-256 file.

`src-tauri/icons/icon.ico` is generated from `src-tauri/icons/logo.svg` with `npx tauri icon src-tauri/icons/logo.svg -o <temp folder>`, then `icon.ico` is copied back.

## Release

Releases do not contain ffmpeg: the app downloads a pinned build into its data folder on first run and checks its SHA-256. To release, raise the version in `src-tauri/Cargo.toml` (`Cargo.lock` follows on the next build) and push to main. The `build` job builds the installer (on main only; other branches build the app alone), signed with the key in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (unsigned, and not released, without them). When the tests and the end-to-end test pass on main and no release for that version exists, the `release` job in `.github/workflows/test.yml` publishes that tested installer with `latest.json`. Every run on main then copies that release's manifest to the `updater` release that the app reads, as `latest.json` and, for installs that were on the old Beta channel, `latest-beta.json`. Keep the private key out of the repository; only the public key is in `tauri.conf.json`.

Only a push to main makes an installer and a release; a branch build stops at the app exe. Installed apps get the release through the in-app updater.

To run a second instance on one PC for testing, set `EZH_DIR` to a folder inside the app data folder (it holds that instance's identity key, recordings, log and WebView2 profile).

To try a change on another Windows PC over SSH without pushing: `scripts/deploy.sh <user@host>`. It sends the working tree, builds, and starts the app.

A deploy builds without an installer (`tauri build --no-bundle`); set `INSTALLER=1` to also build the installer, or `TEST=1` to only run the unit tests (`cargo test` in `src-tauri`). GitHub Actions runs the same tests on every push.

Start the app with `--api` or set `EZH_API=1` to turn the local API on at start. Set `EZH_API_PORT=<port>` to move the local API off its default port 47625 (the end-to-end test gives each instance its own). `src-tauri/src/api.rs` holds the API server and its handlers, which work on a `Backend` trait so that the unit tests run on plain data; the Host check stops DNS rebinding. `src-tauri/src/ops.rs` is the one list of operations that the window and the API both call, and checks that every operation the page or the end-to-end test calls exists and is documented. `docs/llms.txt` is the API reference: it is embedded in the app at build time, served at `GET /llms.txt` and drives the endpoint list in Settings, so change it with the endpoints.

The hardware code is in `src-tauri/src/hw.rs` and the time check in `src-tauri/src/clock.rs`; their tests feed fake PowerShell output and made-up time answers, and `scripts/ui-test.mjs` covers the Where data goes rows and the shared live position, the player's segment scheduling and its shared start with made-up data. In test mode `EZH_NTP=<host:port>` replaces the time server. `node --test scripts/ui-test.mjs` runs the UI's function tests (short Markdown and board cards, sidebar width, the view line and the Live tile problems, the extension status line, the capture buttons, the diagnostics rows), as GitHub Actions does.

The folder export needs ffmpeg: the app uses the one it downloaded, and the end-to-end test makes its tiny test videos with the ffmpeg in its folder. [session-format.md](session-format.md) is the storage format and [protocol.md](protocol.md) the messages; a test fails when a record kind or message the code writes is missing from them.

Set `EZH_RELAY_ONLY=1` to start an instance that uses only the relay, to test the slow path and that no picture crosses it.

Set `EZH_TEST=<port>` for test mode: capture encodes a generated picture and two tones (ffmpeg `testsrc2` and `sine`) instead of the screen, app sound and microphone (a display source gives the test picture and a window source is never found, so it reads as closed), and the window's page accepts DevTools (CDP) connections on that local port. With `EZH_TEST` set, an empty file named `offline` in the instance's data folder cuts its network (it neither dials nor accepts, and its open connections close) until the file is removed, and an empty file named `full` makes its disk read as full.

The end-to-end test `node scripts/e2e.mjs <app exe> <folder with ffmpeg.exe and ffprobe.exe>` starts three test-mode instances on one PC, each with its own data folder (which holds its WebView2 profile) and remote-debugging port, and starts them with `EZH_API=1`; it drives them through `POST /v1/ops/{name}` (the page only where the test is of the page: DevTools evaluates in it). Its steps are named in `scripts/e2e.mjs`. It has three parts that need nothing from each other: `E2E_PART=live` (joining, clocks, the live tiles, a lost log and removing members), `recover` (a crash, a killed encoder, a closed window, a lost connection, a full disk and leaving, during one recording) and `library` (a short recording, then everything else); with none set it runs all three. GitHub Actions builds the app once and runs the three parts as parallel jobs on every push, and keeps the screenshots, states and logs when one fails.

The screen test `npx playwright test -c scripts/screens` opens the page in Chromium with the Tauri core replaced by `scripts/screens/mock.js` (a fixed room, members and recordings at a fixed time) and compares the main screens with the pictures in `scripts/screens/baseline/`; more than 100 changed pixels fail it. GitHub Actions runs it on Linux and keeps the expected, actual and diff pictures when it fails. After an intended change, run it with `--update-snapshots` on Linux (or take the actual pictures from the failed run) and commit the new pictures.

Every case the app is tested against is listed in [test-cases.md](test-cases.md).

A new test or a new case adds its line to `docs/test-cases.md` (a ui-test fails when an end-to-end step is missing there).
