# Deploy pipelines

A *deploy* is an ordered set of scripts that runs for a single project. Scripts
can run on the local machine or on any registered remote machine over SSH.

## Stages

Every deploy script belongs to one of three stages:

| Stage | Runs | Typical use |
|---|---|---|
| `pre` | First, in `order` ascending. | Sanity checks, lock files, backups. |
| `main` | After all `pre` scripts succeed. | Pull, install, build, migrate. |
| `post` | After all `main` scripts succeed. | Cache warmers, smoke checks, notifications. |

Within a stage, scripts run in the `order` field's ascending order. Drag in the
UI to reorder.

Between `main` and `post`, if the project has `autoRestartOnDeploy: true`, the
orchestrator restarts the project's running processes. If there are no `post`
scripts, the restart happens after the last `main` script.

## Failure handling

By default a non-zero exit stops the pipeline immediately. Remaining scripts in
the same and subsequent stages are marked `skipped`.

Mark a script's **Continue on error** to keep going on failure — useful for
optional steps like cache warmers.

A failed deploy:

- Sets the project's `lastError`
- Leaves `lastAttemptedCommit` populated but `lastSucceededCommit` stale
- Triggers the next auto-deploy poll to retry — this is the auto-recovery
  behaviour described in [auto-deploy](#auto-deploy)

## Local vs remote scripts

A script with `machineId: null` (or the default local machine's id) runs on
the local Mac.

A script with `machineId` set to a remote machine runs over SSH. The
orchestrator wraps the command in a small shell preamble that:

1. Prints a marker line with the remote PID (so the orchestrator can kill the
   process group on cancel)
2. `cd`s into the working directory
3. Exports the env vars
4. `exec`s the user's command

The remote machine must have a passwordless SSH key (or be set up in the SSH
agent). See [ssh-remote-machines.md](ssh-remote-machines.md).

## Cancellation

Click **Cancel** during a running deploy. The orchestrator:

- Sends `SIGTERM` to the local script's process group, or
- Issues `kill -TERM <pid>` over SSH for the remote script

The current script is marked failed with error `"Cancelled by user"`. Subsequent
scripts are skipped.

## Auto-deploy

If a project has `autoDeploy: true`, the orchestrator polls its remote git
branch every ~60 seconds:

```
remote_head = git ls-remote origin <branch>
```

If `remote_head` differs from the project's stored `lastSucceededCommit`, the
orchestrator triggers a fresh deploy run. On success the SHA is promoted to
`lastSucceededCommit`. On failure or interruption (e.g. the orchestrator
self-deploy killed mid-pipeline), `lastSucceededCommit` is left stale and the
next poll retries.

To pause auto-deploy without deleting scripts, edit the project and toggle
**Auto-deploy** off.

## Self-deploy caveat

The orchestrator can be configured to deploy itself. The `git pull`, `npm
install`, and `npm run desktop:build` steps all work. The `desktop:install`
step quits the running orchestrator before copying the new `.app` — but since
the deploy pipeline is itself running *under* the orchestrator, killing the
orchestrator also kills the install script. The build artifact ends up in
`src-tauri/target/release/bundle/macos/` but never reaches `/Applications`.

Workarounds:

- Run `npm run desktop:install` from a terminal (not under the orchestrator).
- Or have an external process (launchd, cron, a sibling Mac) do the
  `ditto` and `launchctl kickstart`.

This is a known design limitation, not a bug.

## Example

A Laravel + Vite project deploying to a remote Mac:

```
Stage: main
  1. Git pull            git pull --ff-only
  2. Composer install    composer install --no-dev --optimize-autoloader --no-interaction
  3. NPM install         npm install
  4. NPM build           npm run build
  5. Migrate             php artisan migrate --force
  6. Cache config        php artisan config:cache
  7. Queue restart       php artisan queue:restart
```

Set every script's `machineId` to your production machine. The orchestrator
runs the whole pipeline over SSH and streams output back. Click **Deploy** to
trigger manually, or rely on `autoDeploy: true` to fire on every push.
