# Razer Telemetry & Data Collection Analysis

**Target:** Razer App Engine / Synapse 4.0.827 (build 4.0.827.114)  
**Prepared:** October 26, 2026  
**Scope:** Local application evidence + Razer's published privacy policy and software terms. Not a full product-line audit.

---

### Evidence labels

| Label | Meaning |
|-------|---------|
| **OBSERVED** | Present in saved logs or local filesystem |
| **CODE** | Field, config, or path exists in inspected code |
| **POLICY** | Stated in Razer's public documents |
| **UNKNOWN** | Cannot be established from available evidence |

> **Important:** Capability in code is not the same as proof that every field left your machine.

---

## Severity ranking

Sorted from most concerning to least concerning based on what the software is capable of retaining or transmitting.

### 1. Identifiable analytics stream (High)

**CODE**

The recommendation/analytics library defines a common event schema that can bind activity to an account, session, and device:

- `EventID`, `SessionID`, `UUID` (user identifier)
- `IP` (nullable, but populated via temporary credential service metadata)
- `DeviceFingerprint` (nullable; exact construction not fully established)
- `AgentVersion`, `AppName` / `AppVersion`
- `UnixTimestamp`, `ServiceCode`, `EventCategory`, `EventDimensions`
- `TestData` flag

The uploader uses the authenticated user ID as the AWS Kinesis `PartitionKey`. That is not an anonymous design. A UUID can identify an account even when a real name is absent.

Events are JSON-encoded, batched (up to 500), and submitted via AWS Kinesis `PutRecords`. Local IndexedDB queues can hold pending records on disk.

**Configured production destinations include:**
- `b6y6lf1s29.execute-api.us-east-1.amazonaws.com/sts` — temporary credentials
- AWS Kinesis stream `anne_auth_prod` (us-east-1)
- `bespoke-analytics.razerapi.com` — recommendation service

Upload attempts are scheduled ~3 seconds after start and every 15 seconds while online. A scheduled attempt is not proof of successful transmission every time.

---

### 2. Headset & microphone configuration telemetry (High)

**CODE**

Shared device context supports vendor ID, product ID, edition ID, optional serial/SKU, firmware versions (device + dongle), and connection type (wired / 2.4 GHz / BLE).

**Device Audio Headset** schema can include:
- Volume and mute state
- EQ preset, category, and optional band data
- Game/Chat balance and affected endpoint
- Audio enhancement name, enabled state, level
- Ultra-low-latency setting
- ANC / ambient mode
- In-call mix and do-not-disturb
- Microphone EQ and enhancement settings
- Sidetone enabled state and level
- Power-saving state and idle minutes
- Stereo / THX spatial mode (user or application driven)
- Spatial calibration settings
- Tutorial open / skip events

**Device Microphone** schema additionally covers mic volume/mute, sensitivity, sampling rate, high-pass filter, advanced gain, noise gate, lighting, mixer presets, and related input counts.

These are schema capabilities. They do not prove every control on every headset generates an event, or that every event was uploaded.

---

### 3. Cloud profiles, account linkage & sync (High)

**CODE + POLICY**

Cloud and account endpoints observed in configuration:
- `anne-gors.razerapi.com`
- `wss://pubsub.razersynapse.com/ws`
- `https://pubsub-api.razerapi.com/apikey`
- `id.razer.com`, `razerid.razer.com`, `oauth2.razer.com/services/login_sso`

Razer's software terms state that online Synapse collects warranty details, device type/serial, OS, configuration profiles, installed games, and app usage. Profiles can include keybinds, macros, lighting, power, performance, and EQ settings.

Offline data is described as remaining local while disconnected. Razer ID is said to combine service data for profiling and recommendations. Guest use can still involve technical and usage data. The terms also permit Customer Data use for AI development — this is a policy statement, not proof that any specific local event was used for model training.

Exact cloud profile documents for a given account were not downloaded or inspected.

---

### 4. Broader usage & preference tracking (Medium–High)

**CODE**

Event definitions also cover:
- Login and page-view events (page name + action)
- Profile changes, category, optional profile GUID
- Device inventory (including optional serial numbers)
- Chroma application names/types, toggles, running state, effects, brightness, layers
- Connected Philips Hue light names, IDs, models, on/off state
- Mouse key mapping, sensitivity stages, polling rate
- Global shortcut create/delete and execution counts
- Macro names, optional GUIDs, types, dates, activation counts

Macro names and application names can be revealing on their own. This does not, by itself, establish continuous recording of everything typed into other applications.

---

### 5. Crash reporting & performance diagnostics (Medium)

**OBSERVED + CODE**

- Sentry is initialized for production release 4.0.827 with `tracesSampleRate: 0.01` (1% trace sampling — not 1% of all personal data).
- Ingest host includes `o72136.ingest.us.sentry.io`.
- Child-process / minidump integrations are present.
- A memory-growth reporter can attach machine ID as Sentry user ID, app version, session ID, memory baseline/current/growth, history, active DLLs, window count, uptime, IPC handler count, and optionally DOM / renderer heap data.
- Defaults include 5-minute polling and a 50% growth threshold.

**OBSERVED:** Saved logs indicate memory telemetry was disabled by feature flag. This does not imply all Sentry reporting or other analytics were disabled.

Exact crash contents and successful delivery were not captured. A crash dump can contain process memory; redaction behavior was not inspected.

---

### 6. Local storage on the machine (Medium)

**OBSERVED + CODE**

Primary path: %LOCALAPPDATA%\Razer\RazerAppEngine\User DatatextContains IndexedDB for `apps.razer.com` and `id.razer.com`, Local Storage, Network, Service Worker, Logs, and State directories.

Relevant structures:
- `rzr` / `rzr2` IndexedDB — authentication records keyed by `userId` (rzr2 acts as backup)
- `currentUserId` in localStorage
- `kinesis` IndexedDB — pending analytics events (uploads store + `by_user_id` index)
- Remote-sync SDK local nodes and changes stores
- Installer IndexedDB (installation logs)
- `Logs/extrainfo.json` (machineId passed in login context)
- Rotating application logs (max ~10 MB × 3 files)

Some inter-module stores are RAM-only. Local named pipes carry identity data between components; they are not automatically cloud traffic. Stored identity/cookie databases were not decoded in this review.

---

### 7. Ordinary network / content fetches (Lower)

**CODE**

Hosts such as `apps.razer.com`, `app-assets.razer.com`, and `fwsdk.razersynapse.com` are used for hosted UI, signed feature flags, manifests, and updates. Presence of a download URL is not evidence of personal-data upload.

---

### 8. Policy-level retention statements (Lower / contextual)

**POLICY**

Razer lists categories such as contact details, correspondence, usage/preferences, device information, IP/geolocation, and voluntary survey/promotion data. No fixed Synapse telemetry retention period is published. Retention is described as purpose- and legal-requirement driven. Unneeded personal data is said to be destroyed or irreversibly anonymized; anonymized data may remain.

**UNKNOWN:** Server-side event history, backup lifetime, specific Sentry/Kinesis retention, full downstream recipients, and deletion dates for any particular record cannot be determined from local evidence.

Uninstalling the software or deleting local files does not establish deletion of already-uploaded data. A formal request to Razer would be required for account-linked telemetry, machine/device identifiers, cloud profiles, recipients, and retention schedules.

---

## What this evidence does **not** show

- Continuous microphone recording or conversation capture  
- Camera capture or screenshots  
- Document theft  
- A general keylogger that uploads all typing  
- That every supported schema field is active in every session  
- That session replay was enabled (Sentry replay packages exist in the tree; activation was not established)  
- Full native DLL / service behavior or decrypted live upload bodies  

Microphone-setting telemetry is information about controls, not proof that audio was recorded.

Coverage is incomplete for some native modules. The REA overview noted integrity contradictions on 12 native-module artifacts (labeled observed-untrusted). No native behavior is inferred from those bytes.

---

## Practical summary

The software contains concrete paths for:
1. Identifiable usage analytics  
2. Hardware and configuration detail collection  
3. Cloud profile synchronization  
4. Crash and performance diagnostics  

It can potentially associate what changed, when it changed, which device was involved, and which account/session was active.

The evidence is strongest on **available fields and upload code**. It is weaker on **what actually left the machine** in any given session. A separate network capture would be needed to confirm live destinations and payloads.

---

## Sources

| ID | Reference |
|----|-----------|
| L1 | `evidence/telemetry-storage.md` — telemetry, network, storage report (includes Sentry init and disabled memory telemetry notes) |
| L2 | `evidence/cached-js/background-manager/assets/razer-recommendation-engine-55a6b60f.js` — common event schema, headset/mic/device/Chroma/macro definitions, queue & upload logic |
| L2 | `evidence/cached-js/background-manager/assets/index-b4c3eb99.js` — environment/destination settings and client startup |
| L3 | `evidence/endpoint-inventory.json` — static endpoint inventory |
| L4 | `evidence/rea-engine-js-summary.json` — REA artifact analysis, coverage limits, integrity notes |
| W1 | [Razer Customer Privacy Policy](https://www.razer.com/legal/customer-privacy-policy) (checked 2026-10-26) |
| W2 | [Razer Services & Software Terms of Use](https://www.razer.com/legal/services-and-software-terms-of-use) (checked 2026-10-26) |

No settings were changed and no telemetry was disabled to prepare this report.  
No account credentials or private identifier values are included.