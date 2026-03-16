# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

LAN P2P File Deployer — a Tauri 2.0 desktop app for distributing files across a university lab network (Between 37 and 50 Windows machines on Gigabit LAN). Any machine with the app installed can send files to any subset of other machines (that also have the app installed), specifying the destination path. Transfer uses BitTorrent P2P internally to avoid bottlenecks.

**Core flow:**
1. Sender selects a file and forced destination path (e.g. `C:/Documents/instaladores`)
2. Sender broadcasts availability via UDP or multicast to LAN
3. Receivers (running in background/tray) detect the broadcast and silently download — zero user interaction
4. BitTorrent handles the actual P2P transfer

## Commands

```bash
# Install dependencies
pnpm install

# Development (runs Angular dev server + Tauri)
pnpm tauri dev

# Angular dev server only (port 1420)
pnpm start

# Production build (Angular)
pnpm build

# Full desktop app build
pnpm tauri build

# Angular watch mode
pnpm watch
```

No test files exist yet. Karma/Jasmine is configured for Angular, and `cargo test` can be used for Rust.

## Architecture

**Stack:** Angular 17 (standalone components, TypeScript strict) + Tauri 2.0 (Rust backend) + Tokio async runtime.

**Frontend → Backend IPC:** Angular calls Rust via `invoke()` from `@tauri-apps/api/core`. Commands are registered in `src-tauri/src/lib.rs` with `#[tauri::command]`.

**Key files:**
- `src-tauri/src/lib.rs` — Tauri app setup and command registration (currently empty boilerplate)
- `src-tauri/src/main.rs` — Entry point, calls `lib::run()`
- `src/app/app.component.ts` — Root Angular component with Tauri invoke example
- `src-tauri/tauri.conf.json` — Window config (800×600), dev server URL, bundle settings
- `src-tauri/capabilities/default.json` — Tauri permission model

## Code Rules (from `ai_instructions.md`)

- Strict error handling: use `Result`/`Option`, **no `.unwrap()` in production**
- Comment complex network logic in Spanish
- Keep code modular: separate network, BitTorrent protocol, and Tauri command logic into distinct modules