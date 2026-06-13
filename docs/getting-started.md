# Getting started

This page walks from a fresh install to running your first project in about
five minutes.

## 1. Install

Either download a pre-built `.dmg` from
[Releases](https://github.com/blaze-uz/app-orchestrator/releases) and drag the
app to `/Applications`, or build from source:

```bash
git clone https://github.com/blaze-uz/app-orchestrator.git
cd app-orchestrator
npm install
npm run desktop:install
```

Requirements: macOS 12+, Node LTS, Rust stable, Xcode Command Line Tools.

Open `App Orchestrator.app`. You'll land on the empty Dashboard.

## 2. Add a project

A *project* is a folder with one or more processes.

- Click **Projects** in the left sidebar → **+ New project**.
- Pick a name. Pick a folder (the project's root). Save.

The folder doesn't have to contain anything special — it's just the working
directory for every process in this project unless overridden.

## 3. Add a process

Inside the project, click **+ New process**.

| Field | Example |
|---|---|
| Name | Laravel API |
| Key | `laravel-api` |
| Command | `php` |
| Args | `artisan`, `serve`, `--port=8000` |
| Auto start | ☑ |
| Health check | HTTP, `http://127.0.0.1:8000/up`, expect 200 |

Save. The process appears in the project's sidebar. Click ▶ to start it.

Logs stream into the **Logs** view. Filter by project or process. Click
**Settings** to enable system notifications when a health check flips.

## 4. Open the dashboard

The Dashboard summarizes:

- Total projects / processes
- How many are running, stopped, failed
- Recent activity

It's the screen you leave open while you work.

## 5. Try a deploy script (optional)

Inside a project → **Deploy** → **+ New script**.

| Field | Example |
|---|---|
| Name | Git pull |
| Stage | `main` |
| Command | `git` |
| Args | `pull`, `--ff-only` |

Add a second script ("NPM install" → `npm install`). Drag to reorder.

Click **Deploy** at the project level. The pipeline runs in order, streams
output into the Logs view, and stops on the first failure (unless the script
has *Continue on error* checked).

## 6. Add a remote machine (optional)

Sidebar → **Machines** → **+ New machine**.

- Name: `production`
- Hostname or IP: `prod.example.com` or a Tailscale address
- SSH user: `deploy`
- SSH key path (optional): `~/.ssh/id_ed25519`

Test the connection. If it succeeds, you can now route any process *or* deploy
script to that machine — same UI, the orchestrator runs the command over SSH
and streams output back.

## What next?

- Read [Configuration reference](configuration.md) for the full data model.
- Read [Deploy pipelines](deployments.md) to understand the multi-stage runner.
- Read [Remote machines & SSH](ssh-remote-machines.md) for production-shape setups.
- Enable the [HTTP API](http-api.md) if you want to script the orchestrator from outside.
