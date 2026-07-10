# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

LAN P2P File Deployer (`net-monitor-for-computer-center`) — a Tauri 2.0 desktop app for distributing files across a university computer-lab LAN (Gigabit, ~37–50 Windows machines). Any machine with the app can send a file to any subset of other machines that also have it, with a forced destination path. Transfer uses a homegrown BitTorrent-style P2P swarm protocol over TCP so a single sender doesn't bottleneck the network.

**Core flow (as actually implemented today):**
1. All app instances continuously discover each other via **ARP-table + TCP port probing** (not UDP broadcast — see Networking below).
2. Sender picks a file + forced destination path and selects target peers in the UI.
3. Sender hashes the file into 1 MB chunks (SHA1 each), registers itself as the swarm's seed, and sends a `TransferAnnounce` (over TCP) to every target peer with the full swarm list.
4. Each receiver preallocates the destination file, joins the swarm, and pulls chunks in parallel from whichever swarm peers have them (`ChunkRequest`/`ChunkResponse`), verifying SHA1 per chunk, and re-announces completed chunks to the rest of the swarm (`HaveChunk`) so peers can download from each other, not just the original sender.
5. Receiver runs silently in the background/tray — no user interaction required.

## Commands

```bash
pnpm install          # install deps (pnpm is required — see pnpm-workspace.yaml)
pnpm tauri dev         # run Angular dev server + Tauri desktop shell together
pnpm start             # Angular dev server only, port 1420 (Tauri's devUrl)
pnpm build             # Angular production build
pnpm tauri build       # full desktop app build/bundle
pnpm watch             # Angular build --watch (development config)

# Rust backend (run from src-tauri/)
cargo test             # unit tests for chunker.rs (chunking/hashing) and tracker.rs (swarm tracking)
cargo build

# Angular frontend tests (Karma/Jasmine)
ng test                # only meaningful spec today: file-selector.component.spec.ts
```

There is no CI-enforced lint/format step configured; there's no ESLint/Clippy config checked in beyond the defaults.

## Architecture

**Stack:** Angular 17 (standalone components, signals, TypeScript strict) + Tauri 2.0 (Rust backend) + Tokio async runtime.

### Rust backend (`src-tauri/src/`)

- `lib.rs` — app bootstrap: initializes file+console logging (`tracing`, rotates at 5 MB), builds `AppState`, spawns the TCP listener as a background Tokio task, sets up the system tray (show/quit), registers all `#[tauri::command]`s, and intercepts window close to hide-to-tray instead of exiting. Also contains `get_local_ip()`, which filters out VM/Docker virtual adapters (VirtualBox 192.168.56.x/99.x, Docker bridge 172.17–19.x) to avoid picking an unreachable IP on lab machines that have VM software installed.
- `state.rs` — `AppState`, the single shared app state (`Arc<AppState>` managed by Tauri, injected into commands via `State<'_, Arc<AppState>>`). Holds: `peers` (DashMap of discovered `PeerEntry`), `active_transfers` (DashMap of `ActiveTransfer`), `tracker` (swarm chunk-availability tracker), `config`, and per-transfer cancel channels.
- `network/`
  - `discovery.rs` — just port/version constants. **Note:** `UDP_PORT` is defined but unused; there is no UDP broadcast discovery in the current code despite what `README.md`/`ai_instructions.md` describe. Peer discovery is done entirely by `scanner.rs`.
  - `scanner.rs` — the actual discovery mechanism: reads the OS ARP table, triggers ARP resolution via one-packet pings across the local /24, then TCP-probes a small port range (47833–47843) on each discovered IP to detect whether the app's listener is running there. Runs in three overlapping phases (probe cached ARP entries, ping the subnet, probe newly-discovered IPs) to minimize scan time. Marks peers offline if they disappear from the ARP table.
  - `listener.rs` — binds the TCP listener (default port 47833, auto-incrementing on conflict), accepts inbound connections and hands each off to `transfer::receiver::handle_incoming_connection`.
- `protocol/`
  - `messages.rs` — all wire message types (UDP-shaped types exist but only the TCP ones are actually used): `TransferAnnounce`, `TransferAccepted/Rejected`, `ChunkRequest/Response`, `HaveChunk`, `TransferComplete/Error`. **Inbound dispatch is two-step**: deserialize `TcpMsgEnvelope` (just `msg_type`) first, then re-parse the concrete struct from the same bytes. Do NOT reintroduce a `#[serde(tag = "msg_type")]` enum for this — serde consumes the tag field and the inner structs (which also declare `msg_type` as a required field) fail with "missing field msg_type", silently breaking every incoming message. Unit tests in this file protect the contract.
  - `codec.rs` — custom TCP framing: `[header_len:u32 BE][data_len:u32 BE][header_json][raw_bytes]`. All P2P communication goes through `write_frame`/`read_typed_frame`. Max header 64 KB, max raw payload 1.5 MB (chunk size + overhead).
- `transfer/`
  - `chunker.rs` — fixed 1 MB chunk size (`CHUNK_SIZE`), SHA1 hashing per chunk and whole-file, chunk read/write at byte offset, file preallocation. Has unit tests.
  - `sender.rs` — `start_send`: hashes the file (emitting `hash-progress`), builds the swarm list (self as seed + selected peers), registers all chunks as locally available in the tracker, then TCP-connects to each target and sends `TransferAnnounce` with a 5s connect timeout. The sender is done once announces are delivered — it doesn't push data, it only seeds chunks on request.
  - `receiver.rs` — the core P2P engine: `handle_incoming_connection` dispatches by message type. `handle_transfer_announce` preallocates the file, registers the swarm/tracker, and spawns `run_p2p_downloader`, which pulls all chunks concurrently (bounded by `AppConfig.max_concurrent_chunks`, default 4) using `tracker.best_peer_for_chunk` (random peer holding that chunk, for load balancing) with 3 retries per chunk. On each chunk completion it verifies the hash, writes to disk, updates `ActiveTransfer`, broadcasts `HaveChunk` to the rest of the swarm, and emits `transfer-progress`/`transfer-complete` to the frontend. Also serves chunks to other peers (`handle_chunk_request`) and updates the tracker on `HaveChunk` from neighbors.
  - `tracker.rs` — `TransferTracker`: per-transfer map of `chunk_index -> [peer_id]`, used to pick a download source per chunk.
- `commands/` — thin `#[tauri::command]` wrappers grouped by concern (`peers.rs`, `files.rs`, `transfers.rs`, `config.rs`, `logs.rs`), all registered in `lib.rs`'s `invoke_handler!`. `peers::check_peers_online` does a live TCP-connect check (2s timeout) distinct from the ARP-based scan, used by the UI before actually sending. `logs.rs` tails the tracing log file for the in-app "Registros" tab.
- **Auto-updater**: `lib.rs::check_for_updates` runs at startup via `tauri-plugin-updater`, checking `https://github.com/Coded7Chaos/cc_program/releases/latest/download/latest.json`; if newer, it downloads, installs (NSIS passive mode) and restarts. Updates are minisign-signed: the pubkey lives in `tauri.conf.json` (`plugins.updater.pubkey`), the private key is NOT in the repo (GitHub secret `TAURI_SIGNING_PRIVATE_KEY`, generated with `pnpm tauri signer generate`). Because `bundle.createUpdaterArtifacts` is true, **every** `pnpm tauri build` (locally and in CI) needs `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` env vars set.
- **Release flow**: bump `version` in `src-tauri/tauri.conf.json`, push to main, run the "Release" workflow (`.github/workflows/release.yml`, uses `tauri-apps/tauri-action`) — it creates the `v<version>` tag/release with the signed installer + `latest.json`. `.github/workflows/build-windows.yml` is plain CI (artifacts only, no release).

### Angular frontend (`src/app/`)

- `core/services/tauri-bridge.service.ts` — the **only** place that calls `invoke()` / `listen()`. Every other service/component goes through this. It no-ops or falls back safely when not running inside Tauri (`window.__TAURI_INTERNALS__` check), so the UI can be developed/previewed in a plain browser.
- `core/services/peer.service.ts`, `transfer.service.ts` — Angular **signal**-based state stores that subscribe to backend push events (`peer-updated`, `peer-removed`, `scan-progress`, `transfer-incoming`, `transfer-progress`, `transfer-complete`, `transfer-error`, `hash-progress`) via the bridge and expose readonly signals/computed values to components. There is no NgRx/store library — state lives in these two services.
- `features/` — `file-selector`, `peer-list`, `transfer-monitor`: standalone components consuming the above services.
- Routing (`app.routes.ts`) is currently empty — this is a single-view app, not a routed one.

### Frontend ↔ backend contract

Communication is two-way and event-driven, not just request/response:
- Commands (`invoke`) are for actions/queries (`send_file`, `get_peers`, `refresh_peers`, `cancel_transfer`, etc.).
- Long-running/background state changes are pushed from Rust via `AppHandle::emit(...)` and consumed with `listen()` in `tauri-bridge.service.ts`. When adding a new backend capability that has progress or async completion, follow this emit/listen pattern rather than polling.

### Windows-specific behavior to preserve

- **Firewall rules are created by the installer, not the app**: `src-tauri/windows/installer-hooks.nsh` (wired via `bundle.windows.nsis.installerHooks` in `tauri.conf.json`, with `installMode: perMachine` so the installer elevates) runs `netsh advfirewall` to allow inbound connections to the exe on all profiles, and creates a world-writable `C:\Descargas`. Without that rule, Windows blocks all inbound TCP and discovery/transfers fail silently on every machine. Keep the rule name (`"P2P File Deployer"`) in sync between install and uninstall hooks.
- `get_local_ip()` and `scanner.rs`'s subnet detection actively filter out VM host-only adapters — lab machines commonly have VirtualBox/Docker installed, and picking the wrong adapter breaks P2P silently. The scanner derives the subnet to scan from `state.local_ip` (the already-filtered IP), not from the first interface.
- ARP table parsing in `scanner.rs` has separate branches for Windows/macOS/Linux `arp` output formats; Windows subprocess calls set `CREATE_NO_WINDOW` to avoid flashing a console.
- Closing the window hides to tray instead of quitting (`lib.rs` `on_window_event`); actual exit only happens via the tray "Salir" menu item, which sends a shutdown broadcast to background tasks first.
- Single-instance plugin refocuses the existing window instead of allowing a second process.

## Code Rules (from `ai_instructions.md`)

- Strict error handling: use `Result`/`Option`, **no `.unwrap()` in production** code paths (test code and `sender.rs`'s `.expect()` on Tokio runtime creation are existing exceptions).
- Comment complex network logic in Spanish — this convention is followed throughout `network/`, `protocol/`, and `transfer/` already; match it for new code in those modules.
- Keep code modular: network, protocol (framing/messages), and transfer logic stay in their own modules — don't collapse them back into `commands/` or `lib.rs`.
