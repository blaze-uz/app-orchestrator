<div align="center">

# App Orchestrator

**The macOS panel for every dev process — local and remote.**

Stop juggling 12 terminal tabs. Manage your APIs, queues, schedulers, collectors,
and deploy pipelines from one window. Treat remote Macs as if they were local.

[![CI](https://github.com/blaze-uz/app-orchestrator/actions/workflows/ci.yml/badge.svg)](https://github.com/blaze-uz/app-orchestrator/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/blaze-uz/app-orchestrator?include_prereleases&sort=semver)](https://github.com/blaze-uz/app-orchestrator/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS-lightgrey)](https://github.com/blaze-uz/app-orchestrator/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%20v2-24C8DB)](https://tauri.app)

[Download](https://github.com/blaze-uz/app-orchestrator/releases) ·
[Documentation](docs/) ·
[Contributing](CONTRIBUTING.md) ·
[Security](SECURITY.md)

</div>

---

## Why App Orchestrator?

Most non-trivial backends are *several* processes — a web server, a queue worker,
a scheduler, a few language-specific workers, and often a collector or two. In
development you end up with iTerm split-pane gymnastics or a brittle tmuxinator
config. In production you're SSHing into one server at a time, running deploy
scripts by hand, hoping you remembered them all.

App Orchestrator gives you **one panel** that:

- Starts, stops, restarts every process by project
- Streams stdout/stderr with virtualized rendering — millions of log lines stay snappy
- Runs **TCP / HTTP / custom-command** health checks and auto-restarts failing processes
- Drives **multi-stage deploy pipelines** — same UI for local builds and remote SSH deploys
- Polls `git` and **auto-deploys** when the remote branch advances
- Treats remote Macs as first-class targets — register them, route projects to them, deploy without leaving the app

Built with **Tauri v2** (Rust + React), so it's a real macOS app — fast, native,
no Electron memory footprint.

---

## Screenshots

> 📸 _Screenshots are being re-captured for the public release. In the meantime,
> the project's information density is: a vertical sidebar with collapsible
> projects, a main pane that swaps between Dashboard / Project Detail / Logs /
> Settings, and a unified log viewer with filters._

---

## Install

### Download a pre-built release

```bash
# macOS (Apple Silicon or Intel)
curl -L https://github.com/blaze-uz/app-orchestrator/releases/latest/download/App.Orchestrator_aarch64.dmg -o app-orchestrator.dmg
open app-orchestrator.dmg   # drag to /Applications
```

Or grab the `.dmg` from the [Releases page](https://github.com/blaze-uz/app-orchestrator/releases)
and drag it into `/Applications`. The bundled updater handles future versions.

### Build from source

```bash
git clone https://github.com/blaze-uz/app-orchestrator.git
cd app-orchestrator
npm install
npm run desktop:install
```

Requirements: macOS, Node LTS, Rust stable, Xcode Command Line Tools.
See [CONTRIBUTING.md](CONTRIBUTING.md) for the full setup.

---

## 60-second tour

1. **Add a project** — point it at a folder, give it a name, pick a color.
2. **Define processes** — command, args, env, working directory. Set a health check.
3. **Hit ⌘+R** — the project boots, processes spin up, health checks run.
4. **Watch the logs** — filter by project, process, or log level. Search across millions of lines.
5. **Add a deploy script** — `git pull && composer install && php artisan migrate`. Stage it as `main`. Run it.
6. **Add a remote machine** — SSH user + hostname. Route a process or a deploy script to it. The orchestrator runs your command there over SSH and streams the output back.

Everything is JSON-serializable, importable, and exportable.

---

## How it compares

|                            | App Orchestrator | PM2          | Foreman      | mprocs       | Overmind     |
|----------------------------|------------------|--------------|--------------|--------------|--------------|
| **GUI**                    | macOS native     | CLI only     | CLI only     | TUI          | CLI only     |
| **Multi-machine SSH**      | ✅               | ❌           | ❌           | ❌           | ❌           |
| **Built-in deploy pipelines** | ✅            | ❌           | ❌           | ❌           | ❌           |
| **Auto-deploy from git**   | ✅               | ❌           | ❌           | ❌           | ❌           |
| **Health checks**          | TCP/HTTP/custom  | basic        | ❌           | ❌           | ❌           |
| **Live log streaming UI**  | ✅ virtualized   | terminal     | terminal     | TUI          | terminal     |
| **HTTP API**               | ✅ (opt-in)      | ✅ (PM2 Plus)| ❌           | ❌           | ❌           |
| **Tech stack**             | Tauri/Rust       | Node         | Ruby         | Rust         | Go           |

> If you live in PM2 or tmuxinator and you're happy, stay there. App Orchestrator
> is for the case where you want a single screen across many projects and one or
> more remote build/deploy hosts.

---

## Use cases

- **Polyrepo dev environments** — a Laravel API, a Vite frontend, a Go collector, a Python AI worker, all running on `npm run dev` equivalents, all visible at once.
- **Distributed dev rigs** — one main Mac for the UI, a second Mac for AI workers, a third Mac running queue workers. Drive all three from one window.
- **Production deploys without a Jenkins** — ordered pipeline scripts per project, run over SSH, with cancellation and rollback hooks. Auto-deploy on git push.
- **Always-on background services** — register your queue worker as `autoStart: true`, give it a restart policy and a memory limit, and the orchestrator supervises it across reboots.

---

## HTTP API

The HTTP API is **disabled by default**. When enabled, it binds to `127.0.0.1`
and requires a Bearer token. Every endpoint that drives processes, deploys, or
config is gated. See [docs/http-api.md](docs/http-api.md) for the surface and
[SECURITY.md](SECURITY.md) for the threat model.

```bash
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8765/api/v1/dashboard
```

---

## Documentation

- [Getting started](docs/getting-started.md)
- [Configuration reference](docs/configuration.md)
- [HTTP API reference](docs/http-api.md)
- [Deploy pipelines](docs/deployments.md)
- [Remote machines & SSH](docs/ssh-remote-machines.md)
- [Troubleshooting & FAQ](docs/troubleshooting.md)
- [Architecture](docs/ARCHITECTURE.md)

---

## Built with

- [Tauri v2](https://tauri.app) — Rust-powered desktop shell
- [React 19](https://react.dev) + [TypeScript](https://www.typescriptlang.org/) + [Vite](https://vitejs.dev)
- [Zustand](https://github.com/pmndrs/zustand) — front-end state
- [@tanstack/react-virtual](https://tanstack.com/virtual) — virtualized log rendering
- [Axum](https://github.com/tokio-rs/axum) — HTTP API
- [Tokio](https://tokio.rs) — async process management
- [OpenSSH](https://www.openssh.com) — remote command execution

---

## Roadmap

- [ ] Generic preset framework (load project bundles from `~/Library/Application Support/.../presets/*.json`)
- [ ] Windows and Linux builds
- [ ] Built-in cron-style scheduler
- [ ] Encrypted secret storage (currently relies on macOS Keychain externally)
- [ ] Plugin/extension surface for custom health checks and deploy steps
- [ ] Web UI mode for headless servers

Suggestions welcome — open an [issue](https://github.com/blaze-uz/app-orchestrator/issues/new/choose).

---

## Security

App Orchestrator runs arbitrary local and SSH commands by design. Treat the
HTTP API token like an SSH key. See [SECURITY.md](SECURITY.md) for the threat
model and disclosure process.

---

## Contributing

Pull requests welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup, code
style, and the fork checklist (bundle identifier, updater key, release endpoints).

---

## License

[MIT](LICENSE) © Blaze Uz and contributors.
