# Protocol

What two members send each other, as much as another program needs to talk to the app. Timing, sizes and priorities are policy and live in the code. Records and files are described in [the storage format](session-format.md).

## Transport

iroh QUIC, with the member ID (the public key) as the address. The ALPN is `ez-hanseikai/0`; `ggi-hanseikai/0` is accepted too. Connections are encrypted and authenticated to member IDs.

Each member uses the first relay server of its own list that answers, else the public ones; members with different relays still reach each other. No picture (`v`) crosses a connection whose path is a relay: a `follow` for it ends without data.

A connection from a non-member is closed with "not a member" unless its first request is a `join` answered wait or admitted. One connection serves a pair of members in both directions. Either member may dial, and when two connections exist the one dialed by the lower ID is kept.

## Requests

Each request is one bidirectional stream: the opener writes a JSON object of at most 4 MiB with the field `t` (the message type) and finishes its side; the other side answers on the same stream and finishes. A reply is raw bytes, JSON or text, as listed below. An unknown `t`, or a message that does not apply, gets an empty finished reply. The stream of a message from a peer on a different `proto` (other than `hello`), or from a non-member, ends without an answer.

## Versions

Protocol version: 5

- Every `hello` carries `proto`. From a peer whose `proto` differs, only `hello` is answered; every other message ends without an answer. A peer counts as compatible until its `hello` arrives.
- Bump the version only when an older peer would misread a message. New fields and new record kinds need no bump: readers ignore what they do not know.

## Messages

Fields are in the request; replies are as stated. "ok" is the two bytes `ok`.

| Type | Request fields | Reply |
| --- | --- | --- |
| `hello` | `app` (version text), `proto`, `name`, `room` {`id`, `name`, `members`, `names`}, `set` (the running set {`id`, `room`, `by`, `start_us`, `voice`, `of`} or null), `codecs` (list of `h264`, `hevc`, `av1`), `asks` (list of {`id`, `name`}), `hw` (small hardware object), `have` ({`<set>/<short member id>/<series>`: number of segments held in order}), `logs` ({member ID: length in bytes of that log held}) | The same fields about the answering side. |
| `pull` | `room`, `of` (member ID whose log), `from` (byte offset) | Raw bytes of that log from the offset, up to a size limit, starting at a record boundary and holding at least the first whole record when there is one; the end may fall inside a record. Empty when the room is not shared with the asker. The receiver keeps only whole records, and only at the offset it holds. |
| `follow` | `room`, `owner`, `capture`, `series`, `from` (segment number), `at` (byte offset in that segment), `to?` (last segment), `init` (send `init.mp4` first), `set` (for a recorded set), `live` (follow the live picture; own captures only) | A stream of file frames (below): segments in order from `from`, waiting for an open one and going on into new ones; it ends where the owner has no more, and a live one ends once it is two segments behind the newest. |
| `live` | none | JSON {`sharing`, `capture`, `start_us`, `n` ([newest segment number of `v`, of `m`]), `set` ([set of that segment for `v`, for `m`])}; `sharing` false when the member sends no live picture. |
| `rec` | `action` (`start` or `stop`), `set`, `room`, `by`, and for `start`: `start_us`, `voice`, `of` (for a debrief, the recording it is about, or null) | ok |
| `ping` | none | 8 bytes: the answering side's corrected clock, µs since the Unix epoch, little-endian. The asker measures round trip and clock offset from it. |
| `party` | `room`, `shared` (the shared Watch Party state), `heads` ({member ID: records held}), `typing` | ok |
| `speaking` | `on` | ok. Sent by a recording member only when its voice starts or stops being heard, and with `on` false when it stops recording; the receiver keeps it until the connection closes. |
| `draw` | `room`, `stroke` | ok |
| `check` | none | JSON list of the member's health check lines. |
| `speed` | `bytes`, `pad` (a string that makes the request as large as the test) | `bytes` zero bytes. |
| `report` | none | Plain text diagnostics report with names, addresses and paths removed. |
| `asks` | `room`, `asks` (list of {`id`, `name`}) | ok |
| `answer` | `id`, `approve` | ok |
| `remove` | `room`, `member` | ok |
| `join` | `token`, `name` | JSON `{"ok": true, "room": {...}}` when admitted, `{"wait": true}` while a member decides, `{"declined": true}`, or `{"ok": false}` when the token is not valid. |

`join` is the only message answered before membership. A joiner sends it again every few seconds while the reply is `wait`.

## File framing

`follow` answers with frames: `u32 LE n`, `u32 LE len`, then `len` bytes. `n` 0 is `init.mp4`; `n` 1 and up is that segment. A frame with `len` 0 ends a file. The first frame of a segment starts at the byte offset `at` asked for.

A receiver writes each file as `<name>.part` and renames it when the file is whole. A recorded segment is checked against its `seg` record (size and SHA-256) and `init.mp4` against its `cap` record; a copy that fails is deleted and logged as `file_bad`. Live copies come from their recorder and are kept as they arrive.

## Invite code

`ezh-<inviter member id>-<16 hex characters>`. The hex part is the one-time token sent in `join`. A token works once, only while the inviter's app runs, and a member must approve the joiner.
