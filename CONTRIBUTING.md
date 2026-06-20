# Contributing

Thanks for your interest in Karvon. This document covers local setup,
the dev loop, and what to change if you fork the project to ship your own builds.

## Local setup

Karvon is a cross-platform desktop app — it builds and runs on **macOS and
Windows** from the same source.

Common prerequisites:

- Node.js LTS and npm
- Rust stable + Cargo (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`,
  or [rustup-init.exe](https://rustup.rs) on Windows)

Platform-specific:

- **macOS** — Xcode Command Line Tools.
- **Windows** — the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-studio-build-tools/)
  with the *Desktop development with C++* workload (provides the MSVC linker that
  Rust's `x86_64-pc-windows-msvc` toolchain needs). WebView2 ships with Windows 11
  and recent Windows 10. The OpenSSH client (default on Windows 10 1809+) is
  needed to exercise remote machines and deploys.

```bash
npm install
npm run tauri:dev    # full Tauri app (Rust + React)
npm run dev          # frontend only, with a mock runtime in the browser
```

### Platform abstraction

OS-specific process supervision lives behind one module, `src-tauri/src/platform/`:
`unix.rs` keeps the original POSIX behaviour (`nix` process groups, `ps`/`lsof`),
and `windows.rs` provides the Windows mapping (`CREATE_NEW_PROCESS_GROUP`,
`taskkill /T`, `netstat`/`tasklist`). The rest of the backend calls the
platform-neutral functions in `platform/mod.rs`. The Windows text parsers are
unit-tested on every CI runner (including macOS) so they can be verified without
a Windows machine. When touching process spawning, termination, or discovery,
add the behaviour to **both** impls and keep the function signature identical.

The browser-only mode at <http://127.0.0.1:1420> uses a mock adapter so you can
iterate on the UI without launching native processes.

## Project layout

| Path | Contents |
|---|---|
| `src/` | React + TypeScript frontend |
| `src/lib/api.ts` | Tauri command wrappers |
| `src/lib/mockApi.ts` | Browser-mode mock adapter |
| `src-tauri/src/` | Rust backend |
| `src-tauri/src/commands.rs` | Tauri command handlers |
| `src-tauri/src/http_api.rs` | Optional HTTP API |
| `src-tauri/src/process_manager.rs` | Local process lifecycle |
| `src-tauri/src/platform/` | OS-specific process supervision (`unix.rs` / `windows.rs`) |
| `src-tauri/src/deploy.rs` | Deploy pipeline runner |
| `src-tauri/src/ssh_executor.rs` | Remote command execution |
| `docs/ARCHITECTURE.md` | High-level architecture |

## Build & install locally

```bash
npm run tauri:build        # platform-native bundle in src-tauri/target/release/bundle
```

On **macOS** this produces a signed `.app` + `.dmg`; on **Windows** an NSIS
`-setup.exe` + `.msi` under `bundle/nsis` and `bundle/msi`. `tauri:build` looks
for a minisign signing key at `~/.tauri/karvon.key`; if absent, it skips the
updater artifact so the installers still produce.

macOS also has convenience scripts:

```bash
npm run desktop:install    # build, copy the .app to /Applications, and reopen it
npm run desktop:open       # launch the installed/built .app
```

These two are macOS-only; on Windows, run the generated `-setup.exe`.

## Forking the project

If you publish your own builds, change these before tagging a release:

1. **Bundle identifier** — `src-tauri/tauri.conf.json` (`identifier`) and
   `scripts/desktop-install.mjs` (`bundleIdentifier`). Pick a reverse-DNS name
   you control (e.g. `com.example.karvon`).
2. **Updater public key** — generate a new minisign keypair, replace the
   `plugins.updater.pubkey` field in `tauri.conf.json` with your public key
   (base64), and keep the private key secret.
3. **Updater endpoint** — `plugins.updater.endpoints` and the `latest.json` URL
   in `README.md` point at the original repo. Change them to your fork's
   release path.
4. **Release workflow secrets** — add `TAURI_SIGNING_PRIVATE_KEY` and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repository secrets in GitHub for your
   fork. `…_PASSWORD` may be empty if your key has no passphrase.

## Pull requests

- Run `cargo check` in `src-tauri/` and `npx tsc --noEmit` in the repo root
  before pushing.
- Keep the change focused. If you spot unrelated bugs while working, open
  separate issues rather than bundling them.
- For UI changes, attach a screenshot or short clip.
- For security-relevant changes, see [SECURITY.md](SECURITY.md) for the
  threat-model context.

## Code style

- Rust: standard `rustfmt` (no custom config). Prefer `Result<_, ApiError>`
  over panicking at the command boundary.
- TypeScript: explicit types at public API boundaries; let inference do the
  rest. React function components only.
- No comments restating what the code does — only the *why* when it isn't
  obvious.
