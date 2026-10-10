# Flick on iOS (idea)

> Status: idea only. Nothing here is built. Personal use only; no App Store distribution.

Goal: control how the phone gets used (intentional use, not habit), aggregate usage data with the Mac's [`activity`](activity.md) data, and connect the phone to KOTA. Go as deep into iOS permissions as a self-signed, personal app allows.

## Building blocks

| Layer | What it gives |
|---|---|
| Paid dev account ($99/yr) | One-year provisioning (free accounts expire every 7 days), push, App Groups, the development Family Controls entitlement (no Apple approval needed for personal builds). |
| Supervision (Apple Configurator, wipes the phone) | MDM restrictions: block app install and removal, make the control app non-removable, single-app mode, content filters. Optional self-hosted MDM (NanoMDM or MicroMDM, push cert from mdmcert.download) lets KOTA push policy. |
| Screen Time APIs (FamilyControls, ManagedSettings, DeviceActivity) | Shield apps with a custom block screen, schedules, threshold callbacks. The enforcement engine. |
| Shortcuts "App is opened" automation | Runs on every launch of chosen apps, with no confirmation. Calls an App Intent: log the open to KOTA, show an intention prompt (like one sec), or ask KOTA whether to allow it. The most precise usage hook. |
| "Launcher" | No home screen replacement. Approximate it: all apps in the App Library, one home page of our own widgets, Focus-specific pages, Control Center controls (iOS 18+), Action button, Lock Screen widgets. |
| KOTA bridge | Phone to KOTA over Tailscale. KOTA to phone by silent APNs push. Background work only through BGTaskScheduler or push. |

## Hard limits

- **Usage data is sandboxed.** The DeviceActivityReport extension can show usage but cannot export it (no network, no writes out). App tokens are opaque. Workarounds: Shortcuts app-open events; many DeviceActivity thresholds, each logged to the App Group.
- **No reading other apps.** No notifications, screen content, SMS or call log.
- **No persistent background process.**
- **No jailbreak** on current iOS. Not worth pursuing.
- Android could do all of it (real launcher, UsageStatsManager, notification listener, accessibility service). On iOS, the stack above is the ceiling.

## Mac-side usage data

With Screen Time "Share across devices" on, macOS syncs iPhone usage into Biome (`~/Library/Biome/streams/...`; older versions used `knowledgeC.db`). Flick on the Mac could read it (needs Full Disk Access) and give KOTA cross-device history without the iOS sandbox. The format is undocumented and changes between macOS versions.

## Language: Rust core, thin Swift shell

Swift is required for the Apple-facing parts:

- App Intents: Shortcuts and Siri metadata comes from Swift macros at compile time.
- WidgetKit, Control Center controls, Live Activities: SwiftUI only.
- FamilyControls: the picker is a SwiftUI view; shield and DeviceActivityMonitor are Swift extension targets.

Split:

- **Rust core** (staticlib or XCFramework for `aarch64-apple-ios`, bindings with UniFFI or swift-bridge): KOTA client, event log and store, policy rules, sync. Could be a crate shared with Flick so item ids, store schema and KOTA protocol types match on both devices.
- **Swift shell** (a few hundred lines): app UI, intents, widgets, extensions; each calls into Rust.
- Extensions have tight memory limits (single-digit MB). Keep the Rust linked into them small, or have them only write to the App Group and let the main app do the work.
- Pure-Rust UI (objc2-ui-kit, Dioxus, Tauri mobile) cannot make extensions, widgets or intents. Not useful here.

## First spike

Small SwiftUI app with: a FamilyControls shield, one App Intent that posts app opens to KOTA over Tailscale, and a Shortcuts automation on the 3 to 5 worst apps. That proves the full loop. Then: supervision, the Mac-side Biome reader, the Rust core.

## Open questions

- Which apps to shield, and what the intention prompt asks.
- Does KOTA decide in real time (allow or deny per open), or only set schedules?
- Shared crate layout between Flick and the iOS app.
- Is wiping the phone for supervision worth it, or start unsupervised?
