# Scythe

A lightweight Windows tray app for the **Razer BlackShark V3 Pro**. Battery, volume, Game/Chat balance, and microphone controls in one small dashboard.

Scythe runs locally. No accounts, telemetry, cloud profiles, or updater. Built with Rust and native Windows APIs, without a browser runtime.

![Scythe dashboard](docs/images/dashboard.png)

## Current support

**Version 0.5.0 | Windows x64 | BlackShark V3 Pro**

The BlackShark V3 Pro connected through its USB wireless dongle is the verified configuration. Other headset models, Bluetooth control, and analog connections are not supported. Hardware features may depend on firmware.

| Feature | Status |
| --- | --- |
| Battery percentage and charging | Supported |
| Wireless headset connection status | Supported |
| Estimated battery runtime | Supported after learning discharge behavior |
| Game output volume and mute | Supported through Windows Core Audio |
| Hardware Game/Chat balance | Supported, with center reset |
| Microphone volume and software mute | Supported through Windows Core Audio |
| Physical headset mute detection | Supported |
| Headset mute notifications | Optional Windows alert |
| Microphone EQ | Default, Mic boost, Broadcast, Conference |
| Sidetone | 0-15 |
| Speaker EQ | Current firmware slot displayed; editing not implemented |
| THX, mic enhancements, ANC, sleep timer, firmware updates | Not implemented |
| Other headset models | Not yet supported |

## Using Scythe

Build the app below and run `target/release/scythe.exe`. It starts in the Windows notification area; the icon may be inside the hidden-icons menu.

- Hover for a compact battery, microphone, and Game-volume preview.
- Click the tray icon to open the dashboard.
- Drag the custom header to move the borderless window.
- Minimize, close, or press Escape to hide the dashboard while Scythe stays running.
- Right-click the tray icon and choose Exit to quit.

Route games to the headset's **Game** output and voice chat to **Chat**. The hardware mixer leaves both channels at full level in the center; moving toward one side lowers the other channel. The Game volume slider separately controls the Windows Game endpoint.

Software microphone mute and physical headset mute are separate. Release a physical mute using the headset button. Enable **Headset mute notifications** for an alert when the physical button changes from unmuted to muted. Alerts default off; Windows notification settings may suppress them.

Battery glow is green at 60?100%, yellow at 20?59%, and red at 0?19%. Unknown readings stay neutral. Runtime estimation requires at least 20 minutes and a drop of 3 percentage points. Charging, a percentage increase, missing data, or exit resets learning. Estimates are approximate; no battery history is saved.

The dashboard uses privately embedded Clear Sans, antialiased native drawing, and a blue/navy/cream palette. Fonts are not installed globally.

## Build

Requirements: Windows x64, stable Rust with the MSVC toolchain supporting edition 2024, and Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK.

```powershell
git clone https://github.com/havioo/scythe.git
cd scythe
cargo build --release --locked
.\target\release\scythe.exe
```

Cargo downloads dependencies on the first build. The app itself works offline. The release statically links the C runtime and uses Windows system libraries. Scythe does not replace audio drivers and does not use Synapse as its control backend. Working headset drivers and Windows audio endpoints are still required.

When distributing a build, include `LICENSE` and the font license and attribution from `assets/fonts` alongside the executable.

## Settings and privacy

The notification preference is stored in `%LOCALAPPDATA%\Scythe\settings.json`. If absent, preferences can be read from the former `BlackSharkLocal` folder. No account, telemetry, or stored device history. Diagnostics only produce output when explicitly requested.

## Development

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Twelve tests cover report checksums and framing, invalid packets, physical mute notifications, notification transitions, mixer endpoints, battery colors, discharge estimates, private font loading, and antialiased edges at 1x and 2x. Protocol fixtures contain packet bytes and direction labels rather than full session logs. Unit tests do not establish compatibility with untested hardware.

| Module | Purpose |
| --- | --- |
| `src/device.rs` | HID enumeration, transport, hardware commands |
| `src/protocol.rs` | Framing and report validation |
| `src/runtime.rs` | Polling, notifications, action handling |
| `src/audio.rs` | Windows audio endpoint controls |
| `src/mic.rs`, `src/mixer.rs` | Mute events and Game/Chat behavior |
| `src/battery.rs`, `src/trend.rs` | Battery colors and runtime estimation |
| `src/ui.rs`, `src/render.rs` | Dashboard, tray preview, native rendering |
| `src/settings.rs` | Preferences |

### Diagnostics

```powershell
.\target\release\scythe.exe --probe
.\target\release\scythe.exe --snapshot
.\target\release\scythe.exe --renderer-info
.\target\release\scythe.exe --window-info
.\target\release\scythe.exe --render dashboard.bmp
.\target\release\scythe.exe --render-hover preview.bmp
.\target\release\scythe.exe --render-2x dashboard-2x.bmp
```

`--probe` enumerates HID descriptors and may include local device identifiers. `--snapshot` reads current state. Render modes export the native painter to a bitmap. `--window-info` inspects an existing Scythe window; `--show` opens the dashboard on launch.

`--verify-mic` and `--verify-mixer` **write current settings back to the device** and check readback. These developer diagnostics are for supported hardware, not discovery on unknown models.

## More headset models

If I have the time and passion to keep expanding this, I'd like to support more headsets. Anyone else is welcome to fork Scythe and add their headset's model implementation. Pull requests are welcome too.

See [CONTRIBUTING.md](CONTRIBUTING.md) for device identification, model-specific commands, endpoint matching, fixtures, and hardware validation.

## Known limitations

Only the BlackShark V3 Pro dongle configuration is verified. Device selection and audio endpoint matching are model-specific; a generic profile system is not implemented. Reconnect, sleep/wake, power transitions, long discharge sessions, and mixed-monitor DPI need broader testing. Keyboard navigation is supported, but custom controls do not expose complete screen-reader semantics.

Scythe is an independent project, not affiliated with or endorsed by Razer. Product names belong to their respective owners.

## License

Scythe is [MIT licensed](LICENSE). Bundled Clear Sans retains its separate [Apache 2.0 license](assets/fonts/LICENSE.txt) and [attribution](assets/fonts/README.md).
