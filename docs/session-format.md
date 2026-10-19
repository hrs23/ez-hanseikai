# Session folder

A recording can leave the app as a folder of plain files and come back in as a recording. The format is made of common files only: video (mp4 with H.264 and AAC, or whatever the file already has), JSON, WebVTT and CSV. Anyone can make a folder by hand or with any tool, and the folder opens directly in a video editor.

File > Import recording… reads a folder or video files; Recording > Export to folder… writes a folder from any recording; the [local API](api.md) does both too. The app's own storage does not change: this is only the way in and out.

## The smallest session

A folder with video files is already a session: the folder name is the title, each file is one view named after the file, and the start comes from the files' times.

```
Scrim night/
  Ann.mp4
  Ben.mp4
```

`session.json` adds only what cannot be derived. Its smallest form:

```json
{
  "title": "Scrim vs Team B"
}
```

## The full form

```
Scrim vs Team B/
  session.json
  Ann.mp4
  Ann voice.m4a
  Ann.vtt
  Ben.mp4
  notes.csv
  chapters.vtt
```

```json
{
  "id": "2026-10-19 21-30-00",
  "title": "Scrim vs Team B",
  "start": "2026-10-19T12:30:00Z",
  "views": [
    {"name": "Ann", "file": "Ann.mp4", "voice": "Ann voice.m4a"},
    {"name": "Ben", "file": "Ben.mp4", "offset_ms": 2500}
  ],
  "events": [
    {"at_s": 84.5, "text": "Late rotate, the ring was closing", "tags": ["mistake", "rotate"]},
    {"at_s": 120, "kind": "mark"},
    {"at_s": 0, "to_s": 612.3, "text": "Match 1"}
  ]
}
```

Every field is optional. Unknown fields are ignored.

| Field | Meaning | Default |
| --- | --- | --- |
| `id` | The name the recording had where it came from. Only informational. | none |
| `title` | The name shown in the recording list, at most 80 characters. | the folder name |
| `start` | When the recording started: ISO 8601 UTC, `2026-10-19T12:30:00Z` (milliseconds allowed). | the earliest file start (see below) |
| `views` | One entry per picture, at most 6. | one view per video file in the folder |
| `views[].name` | The member name shown on the view, at most 24 characters. | the file name without its extension |
| `views[].file` | The video (or sound-only) file, relative to the folder, no `..`. | `<name>.mp4` |
| `views[].voice` | A separate sound file for the voice of that view. | none |
| `views[].offset_ms` | How many milliseconds after `start` this file begins, 0 to 86400000. | 0 |
| `events[]` | Notes, marks and chapters. | none |

An event has `at_s`, plain seconds from `start`, and optional `text`, `tags` (words, `#` optional) and `to_s`. With `to_s` it is a chapter (it needs `text`); without `text` it is a mark; otherwise it is a note. `kind` (`note`, `mark` or `chapter`) can say it outright. A note is at most 500 characters.

## Other files

- `notes.csv`: notes and marks, one per row. The first row names the columns: `seconds` (also `time` or `at_s`; a number such as `84.5` or a clock such as `1:24.5`), and optionally `kind` (`note` or `mark`), `text` and `tags` (words separated by spaces). A spreadsheet can write it.
- `chapters.vtt`: chapters as WebVTT cues; the cue text is the chapter name, the cue times are seconds from `start`.
- `<video file name>.vtt` (for example `Ann.vtt`): subtitles for that view as WebVTT cues. They show as subtitles on that view.

Notes, chapters and subtitles from `session.json`, `notes.csv`, `chapters.vtt` and the `.vtt` files are all read, and all times are seconds from `start`, whichever view they are about.

## Import

- Choose a folder or video files in File > Import recording…, or drop them on the window. The dialog shows the title and the name of each view, which can be changed. The recording needs an open room.
- A folder with `session.json` follows it; a folder without one is the smallest session above. Files chosen one by one are one view each; a single file is a recording with one view.
- With no `start`, the start is the earliest file start: the `creation_time` inside the file when it has one, otherwise the file's modification time minus its length. Without `offset_ms` every view starts together.
- A file whose picture the app can already play (H.264, and HEVC or AV1 when every member can play them; VP9) is not re-encoded; AAC, MP3, Opus and FLAC sound is kept. Anything else is converted to H.264 and AAC. A file with sound only (m4a, mp3, wav…) is a view with voice only.
- An imported recording is an ordinary recording: it is in the list with the Imported mark, plays in the player and in Watch Party, takes notes, marks, drawings and events, and the plugins read it (Transcription reads the sound of a view that has no separate voice). The room shares it like a recorded one: the importer holds it, everyone receives it, and the importer is named on the recording.
- An import always makes a new recording; the same folder imported twice makes two. A problem in `session.json`, a note row or a WebVTT file stops the import with the reason and imports nothing.

## Export

Recording > Export to folder… (or the recording's menu) asks for a place and writes a new folder named after the recording.

- One plain `.mp4` per view, with its picture and app sound, and, when a view has a separate voice, `<name> voice.m4a`. A view that has only voice is `<name>.m4a`. Nothing is re-encoded when the view is one continuous part.
- `session.json` with `id`, `start`, and the views with their `offset_ms`, so every file keeps its place on one timeline: the file starts `offset_ms` after `start`.
- `notes.csv` for the notes and marks, `chapters.vtt` for the chapters (chapters people made, and the matches that the Apex matches plugin found), and `<name>.vtt` for the spoken lines of Transcription.
- Start every video together: instead of leaving offsets in `session.json`, each file that begins late is padded with black and silence at the start, so all offsets are 0 and the files line up in an editor with no manual syncing.

## Editing in other software and coming back

1. Export, with or without aligned starts.
2. Open the files in any editor and cut, colour, add or replace the mp4s. Keep the file names, or change them in `session.json`.
3. Import the folder again. Missing optional parts (`voice`, `notes.csv`, `chapters.vtt`, subtitles) are fine, and `session.json` is read again, so a trimmed video only needs its `offset_ms` changed. The result is a new recording in the room: a walkthrough or a highlight reel that everyone can watch together, with notes.
