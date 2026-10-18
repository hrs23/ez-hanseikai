# EZ Hanseikai

Team reviews made ez.

A Windows app for recording and reviewing team games together. Each player records their own screen and game sound; recordings and notes reach the others while they are still recording, so the team can review the last match while the next one records.

Work in progress. See [the design](docs/design.md).

## Principles

- Simple: as few settings, screens and features as the job needs.
- Never touches the game: it only captures the screen and the game's sound, never injects code, draws overlays or reads game memory.
- No cloud: recordings and notes go straight to your teammates and stay on your PCs.
- Nothing is lost by one person's delete: everyone keeps their own copy.
- Analysis is done by plugins that only read the recordings; each can be turned on or off on its own.
- Free and open source.

## Install

1. Download `EZ.Hanseikai_<version>_x64-setup.exe` from [Releases](https://github.com/hrs23/ez-hanseikai/releases) and run it. It is unsigned, so SmartScreen warns: More info, then Run anyway. It installs for your Windows user only and needs no admin rights.
2. Start the app. Allow it through the firewall when Windows asks. On first run it downloads ffmpeg.
3. Enter your name. One player presses Create a room and sends the invite code; the others paste it and press Join (any member can create more invites). Then press Record.

Requires Windows 10 version 2004 or later, or Windows 11.

## Update

The app checks for a new version at start and installs it when you agree. Updates are signed and checked before they install. To go back to an older version, uninstall (keeping your data) and install that version from Releases.

## Uninstall

Settings > Apps > Installed apps > EZ Hanseikai > Uninstall. Your data stays unless you tick the option to delete it, so a reinstall keeps your ID and recordings.

Data lives in `%LOCALAPPDATA%\dev.ez-hanseikai.app`; the app's Files section opens it and can reset it. `key.bin` is your identity: keep it private, and deleting it changes your ID.

## Security

- Only the teammates you connect to receive your recording, game sound, notes and name. Anything visible on your main screen, such as chats or notifications, is recorded too.
- Your ID is the public half of a key made on first start. Connections are encrypted and accepted only from room members. An invite code works once; anyone who joins can invite others, so share it only with your team.
- The Share switch in the settings stops others from receiving your recordings; the speed limit caps how fast recordings move.

## Development

See [docs/development.md](docs/development.md).

## License

MIT or Apache-2.0, at your option.
