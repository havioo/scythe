# Protocol fixtures

BlackShark V3 Pro report bytes and direction labels only; timestamps, paths, process data, and session metadata are omitted.

- `headset-reports.json`: 249 reports, including queries, replies, notifications, and Game/Chat writes at 0, 10, and 20.
- `microphone-reports.json`: physical mute/unmute events and microphone EQ/sidetone writes on updated firmware.

Tests consume `normalized_result.messages[].payload.kind` and `.bytes`. Byte arrays contain original complete reports, including checksums.
