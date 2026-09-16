# Quota Panel

[中文](README.md) | **English**

An always-on-top desktop widget for macOS / Windows that monitors AI usage quotas.
A Dynamic-Island-style pill sits at the top of the screen; clicking it expands a card showing
**Factory (Droid), Devin and Cursor (including Grok Bot)** side by side.

Rust + Tauri v2. The UI is a single dependency-free HTML file. No Electron.

---

## What it shows

| Source | Card contents | Credentials read from |
| --- | --- | --- |
| **Factory (Droid)** | 5-hour / 7-day / 30-day usage windows, Core free-model pool, prepaid balance | locally logged-in `droid` CLI |
| **Devin** | daily / weekly used percentages and their reset times | locally logged-in Devin Desktop / CLI |
| **Cursor** | Auto / API / **Grok Bot** quota pools, combined usage, billing-cycle reset | locally logged-in Cursor desktop app |

About Grok Bot: it is **a Cursor product**. Its weekly quota is served by Cursor's own
`GetSandUsageStatus` endpoint and billed against the Cursor session, so it is shown inside
the Cursor card rather than as a separate card.

The pill only shows the **tightest percentage across all sources**, coloured by threshold
(yellow at 70%, red at 90% by default; thresholds live in `AppConfig` in `models.rs`).

## Devin: how the data gets here

Read this section first if the Devin panel is the one you care about.

- **Live on every refresh.** Each refresh cycle calls Cognition's seat-management endpoint
  (`server.codeium.com/.../SeatManagementService/GetUserStatus`, protobuf over Connect-RPC)
  directly. There is **no local-cache fallback**: if the call fails, the card shows an explicit
  error instead of silently displaying stale numbers.
- **Cadence.** Auto-refresh every 5 minutes, plus manual refresh via the ↻ button or the tray
  menu (instant). It is polling, not a push feed, so values can be up to 5 minutes old.
- **Prerequisite.** You must be logged into Devin Desktop (or the Devin CLI) on the same machine.
  The app reads the existing login token from `~/.config/devin/credentials.toml`,
  `~/.codeium/windsurf/credentials.toml` or Devin Desktop's `state.vscdb` — **read-only**,
  never stored or uploaded. If the session expires (HTTP 401/403) the card says so; log in again.
- **Maturity caveat.** The endpoint and its protobuf field numbers are reverse-engineered and
  unofficial. Cognition can change them at any time; when that happens the panel shows an error
  rather than wrong numbers. The integration works today but is not yet mature — try it and
  report what you see.

## Prerequisites (read first)

The app **stores no passwords and has no login UI**. It only reads credentials of clients you
have already logged into on this machine (every file is opened read-only). Therefore:

- Log into the corresponding client on this machine for each source you want to see:
  Cursor desktop, Devin Desktop or CLI, `droid` CLI.
- A source you are not logged into shows "read failed" on its card **without affecting the
  others** — the three sources are fetched concurrently and independently.
- If within the Cursor card only Grok has numbers while Auto/API show "no data" (or vice
  versa), one of the two endpoints failed on its own; the card footer states the reason.

Exact read locations (all read-only; source in parentheses):

- `~/.factory/auth.v2.loginkeychain` or `auth.v2.keyring`, plus the `auth-encryption-key`
  entry in the macOS Keychain / Windows Credential Manager (Factory) — the file is AES-256-GCM
  ciphertext, decrypted in memory only; the key never touches disk
- `~/.config/devin/credentials.toml`, `~/.codeium/windsurf/credentials.toml`, or Devin
  Desktop's `state.vscdb` (Devin)
- Cursor desktop's `state.vscdb` and `~/.cursor/cli-config.json` (Cursor and Grok Bot)

## Build & run

Requirements:

- [Rust stable](https://rustup.rs)
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- Windows: MSVC build tools + Windows SDK; WebView2 runtime (preinstalled on Windows 10/11)

```bash
cd quota-panel-tauri/src-tauri

cargo run                # development mode
cargo build --release    # release binary in target/release/
```

On macOS you can run `target/release/quota-panel-tauri` directly; for `.app` / `.dmg` / `.msi`
bundles use `cargo tauri build` (requires
[tauri-cli](https://tauri.app/start/prerequisites/)).

## Usage

- After launch a pill appears at the top of the screen with the tightest percentage and a status dot.
- **Click the pill** to expand the card; **click ✕** at the card's top right to collapse it.
- **↻ button**: refresh immediately.
- **Tray icon** (menu bar): left-click shows and focuses the window; the right-click menu has
  "Refresh quotas now" and "Quit Quota Panel".
- Data refreshes **every 5 minutes** by default; the card footer shows the last update time.
- **Language follows the OS**: a Chinese system locale renders the UI, the tray menu and all
  error messages in Chinese, anything else renders English. There is no in-app switch.

## Known limitations

- **No persisted configuration**: refresh interval and colour thresholds are hardcoded
  (5 min / 70% / 90%); there is no config file and no settings UI — change the source to change them.
- All endpoints are **unofficial internal APIs**; a vendor can change one at any time and that
  column will stop working. When that happens you get a concrete error, never fake data.
- The macOS build enables `macOSPrivateApi` (required for the frameless transparent window),
  so it **cannot be submitted to the Mac App Store**.

## Risks & disclaimer

Please read this before using or redistributing.

- The tool calls **unofficial internal APIs** of Cursor / Devin / Factory and reads their local
  login state. These actions **may violate the vendors' terms of service**; the risk (including
  possible account restrictions) is borne by the user.
- Every outbound request identifies itself honestly as `quota-panel/<version>`; it **never
  impersonates a vendor client**.
- The app only reads local credentials; it never writes, uploads or stores any token. Network
  traffic goes only to the vendors' own domains (`cursor.com`, `api2.cursor.sh`,
  `server.codeium.com`, Factory's API). Local databases of Cursor / Devin are opened in
  SQLite read-only mode.
- Grok Bot percentages are converted from dollar quotas server-side by Cursor with an
  **undisclosed denominator** — treat them as a trend, not an exact remaining balance.
- This project is not affiliated with Anysphere (Cursor), xAI, Cognition (Devin) or Factory.
  Cursor, Grok Bot, Devin and Factory / Droid are trademarks of their respective owners.

## Development

```
quota-panel-tauri/
├── src-tauri/
│   └── src/
│       ├── lib.rs           # app entry, tray, background polling, 3-way concurrent scheduling
│       ├── cursor.rs        # Cursor usage-summary + Grok Bot GetSandUsageStatus
│       ├── devin.rs         # Devin (protobuf over Connect-RPC)
│       ├── factory.rs       # Factory / Droid
│       ├── credentials.rs   # read-only local credential loading (incl. AES-GCM decryption)
│       ├── http.rs          # shared client UA + bounded retries
│       ├── commands.rs      # Tauri IPC commands
│       └── models.rs        # data structures shared between backend and UI
└── ui/index.html            # the entire UI: single file, zero external dependencies
```

Measured on macOS (including WebKit's GPU / WebContent / Networking child processes):
about **150 MB RSS** idle. For comparison, the Electron version it replaced used ~470 MB.
