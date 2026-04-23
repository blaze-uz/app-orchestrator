# Local Project Orchestrator

macOS desktop app for managing local projects and their processes from one orchestration panel.

## Stack

- Tauri v2
- React + TypeScript + Vite
- Zustand
- Virtualized log rendering with `@tanstack/react-virtual`
- JSON config persistence in the Tauri app config directory

## Current MVP

- Workspace/project/process domain model
- Project CRUD and process definition CRUD
- Process start/stop/restart command surface
- Project-level start/stop/restart and restart failed
- Event-driven runtime state and live logs
- Basic TCP/HTTP/custom-command health check architecture
- Dashboard, projects, project detail, process detail, logs, settings
- JSON import/export with secret-like env redaction
- Browser mock runtime for UI development without Tauri

## Development

```bash
npm install
npm run dev
```

Open `http://127.0.0.1:1420/`.

The browser version uses a mock runtime so the UI can be tested without launching native processes.

## Tauri

Tauri requires Rust and Cargo:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
npm run tauri:dev
```

On macOS, Xcode Command Line Tools are also required.

## Build

```bash
npm run build
npm run tauri:build
```

`npm run build` validates TypeScript and produces the frontend bundle. `npm run tauri:build` additionally compiles the Rust native layer and bundles the macOS app.

## Desktop runner

```bash
npm run desktop:build
npm run desktop:install
npm run desktop:open
```

`desktop:install` builds the signed updater bundle, quits a running copy of Local Project Orchestrator, copies the `.app` into `/Applications`, and reopens it. User data is stored in the Tauri app config directory, outside the `.app` bundle, so replacing the app does not remove projects, settings, or activity.

## Releases and updates

The app uses Tauri's signed updater and reads release metadata from:

```text
https://github.com/blaze-uz/app-orchestrator/releases/latest/download/latest.json
```

The updater public key is committed in `src-tauri/tauri.conf.json`. Keep the private key secret. For local builds, the expected key path is:

```text
~/.tauri/local-project-orchestrator.key
```

For GitHub Releases, add these repository secrets:

```text
TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` can be empty when the private key was generated without a password. Use `npm run version:set -- 0.1.1` before tagging a release so `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` stay in sync.
