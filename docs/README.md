# App Orchestrator documentation

Welcome. App Orchestrator is a macOS desktop control panel for orchestrating
local and remote development processes. This directory holds the canonical
documentation.

## Start here

| Guide | When to read |
|---|---|
| [Getting started](getting-started.md) | First-time setup and a 5-minute tour. |
| [Configuration reference](configuration.md) | Field-by-field reference for `config.json`. |
| [HTTP API](http-api.md) | Drive the orchestrator from scripts and dashboards. |
| [Deploy pipelines](deployments.md) | Define ordered scripts that run locally or over SSH. |
| [Remote machines & SSH](ssh-remote-machines.md) | Register a remote Mac and route work to it. |
| [Troubleshooting & FAQ](troubleshooting.md) | The "why isn't this working" page. |
| [Architecture](ARCHITECTURE.md) | How the layers fit together. |

## External pages

- [README](../README.md) — what App Orchestrator is and how it compares.
- [CONTRIBUTING](../CONTRIBUTING.md) — local setup, fork checklist, PR conventions.
- [SECURITY](../SECURITY.md) — threat model and vulnerability disclosure.
- [Releases](https://github.com/blaze-uz/app-orchestrator/releases) — download
  the latest `.dmg`.

## Concepts at a glance

- **Workspace** — top-level grouping. You usually have one.
- **Project** — a folder on disk plus a set of processes and deploy scripts.
- **Process definition** — command + args + env + cwd + health check + restart policy.
- **Deploy script** — one step in an ordered pipeline. Has a `stage` (pre / main / post).
- **Machine** — local (default) or remote (SSH). Processes and deploy scripts can target any machine.
- **Auto-deploy** — poller compares the project's git working tree to its
  remote and runs the deploy pipeline when the remote moves ahead.
