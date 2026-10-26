# Adding headset support

Fork Scythe, create a branch for your model, and open a pull request with the exact model, firmware, connection type, verified features, and known limitations. Small contributions are welcome.

## 1. Identify the device

Build the app and run `scythe.exe --probe`. Compare results with the headset connected and disconnected. Record vendor/product IDs, interfaces, usage page/usage, and report lengths. Distinguish dongle presence from an actual wireless headset connection.

Keep serial numbers, local device paths, usernames, and full logs out of public commits and issues.

## 2. Select a model implementation

Start in `src/device.rs`. Enumeration currently recognizes product IDs `0x0576` and `0x0577`. Control selection expects interface 5, usage page `0xff14`, usage 1, and 64-byte input/output reports. These are model-specific values.

Introduce explicit model/profile selection before enabling another device. A profile should define identity, control collection, capabilities, framing, routing, commands, and value ranges. Keep BlackShark behavior intact. Adding a product ID alone does not make BlackShark commands compatible with another headset.

## 3. Match audio endpoints

Review `src/audio.rs`; endpoint matching currently uses BlackShark-specific names. Associate output/input endpoints with the selected headset, preferably through device identity. Verify volume and software mute affect only the intended endpoint.

Game/Chat requires separate audio endpoints or a verified hardware mixer. Keep unsupported controls unavailable rather than assuming the feature exists.

## 4. Verify the hardware protocol

Use documentation or observations of your own device and official controls. Change one setting at a time and compare request/response pairs. Establish report IDs, lengths, checksums, routing, response matching, notification formats, and bounded timeouts.

Start with read-only battery, charging, and connection queries. Check ranges and distinguish missing data, disconnected, and zero battery. Confirm mute polarity against the physical button. A response alone is insufficient; check readback and actual behavior.

Only implement writes after confirming commands and ranges. Do not discover commands using firmware flashing, reset, power-off, or arbitrary command sweeps.

## 5. Connect supported features

- `src/protocol.rs`: framing and validation where model differences require it.
- `src/device.rs`: model-specific queries and setters, with acknowledgement/readback.
- `src/runtime.rs`: polling, notification routing, and actions.
- `src/mic.rs` and `src/mixer.rs`: notification interpretation and value mapping.
- `src/main.rs` and `src/ui.rs`: state and capability-dependent controls as needed.

Keep hardware I/O off the UI thread, unknown readings unknown, and physical mute separate from software mute. Do not expose unsupported EQ, sidetone, or mixer controls.

## 6. Add fixtures and run checks

Add necessary protocol bytes and direction labels under `tests/fixtures`. Exclude vendor binaries/source, complete session logs, and personal identifiers. Document model, operation, and expected values. Preserve original expected packets rather than generating them with the implementation being tested.

Test framing, corrupt/truncated reports, ranges, notifications, and model differences. Run:

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

## 7. Test on the actual headset

Verify battery/charging, wireless on/off, unplug/replug, sleep/wake, volume/mute, physical mute notifications, and each exposed feature. Check Game/Chat independently, including center and both endpoints. Start audio testing at a comfortable volume.

Include hardware results and remaining limitations in the pull request, and update the README support table. A successful build alone does not establish headset support.

## License

Contributions must be compatible with MIT. Preserve third-party notices, including the separate Apache 2.0 license for Clear Sans.

## UI changes

Keep interaction rectangles and painting aligned across DPI scales. Test connected and disconnected states, keyboard navigation, and staged EQ changes when the active slot changes. The sidebar has five sections; page transitions must respect Reduce Motion and Windows animation preferences. Custom sleep values must remain whole minutes within the confirmed firmware byte range. Do not expose RGB or effects modes without a verified command and hardware behavior.
