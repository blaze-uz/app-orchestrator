# Troubleshooting & FAQ

The most common problems, with the fixes that have actually worked.

## "Address already in use" on the HTTP API

You enabled the API on a port something else holds. Either:

- Pick a different port in **Settings → Config → HTTP API**, or
- Find and kill the holder: `lsof -i :8765` and `kill <pid>`.

## A process won't start — "Command not found"

The orchestrator runs commands with a *clean* PATH. By default it includes
`/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`, `/usr/sbin`,
`/sbin`, and `$HOME/Library/Application Support/Herd/bin` (for Laravel Herd
users). Anything else needs to be added explicitly.

Fix it one of two ways:

- Set the full path in the command: `/opt/homebrew/bin/php` instead of `php`.
- Add `PATH=/your/custom/path:$PATH` to the process's env.

## "Direct launch failed" then "shell fallback failed"

The orchestrator tried to `execvp` the command, got `ENOENT`, fell back to
`/bin/zsh -lc 'exec <command>'`, and that failed too. Causes:

- Misspelled command.
- A `~`/`$HOME` in the command itself — the direct launch path doesn't expand
  shell variables. Use the full path or rely on the shell-fallback case by
  starting your command with `$` or `~`.

## Health check keeps failing

Open the process detail. The health check status panel shows the last attempt
and the failure reason.

- **TCP**: nothing's listening on `host:port` yet. Increase `startupDelayMs`
  or wait longer between retries.
- **HTTP**: wrong URL, wrong expected status, or the server is binding to a
  different host than you check (e.g. `0.0.0.0` vs `127.0.0.1`).
- **Command**: the command's exit code isn't 0. Run it manually with the same
  env and see why.

## Auto-deploy keeps retrying

Auto-deploy compares `lastSucceededCommit` with `git ls-remote`. If they
differ, it triggers. Common causes for a permanent retry loop:

1. **Force-pushed history**: a `git pull --ff-only` step in your deploy
   pipeline now fails because the local clone is on a discarded branch. Fix
   it on the machine: `git fetch && git reset --hard origin/main`.
2. **Self-deploy install fails**: see the [self-deploy caveat in
   deployments.md](deployments.md#self-deploy-caveat).
3. **A required tool isn't on PATH**: the build step exits non-zero. Inspect
   the deploy logs.

To stop the loop without fixing the underlying issue, disable auto-deploy on
the project: edit it and toggle the field off.

## Remote SSH command works manually but fails through the orchestrator

The orchestrator uses `BatchMode=yes` — no password prompts, no interactive
key passphrases. If your `ssh user@host` works only because your terminal
asks you for a key passphrase, the orchestrator will fail.

Fixes:

- `ssh-add ~/.ssh/id_ed25519` to put the key into the running agent.
- Or set `sshKeyPath` on the machine record so the orchestrator passes
  `-i` and `-o IdentitiesOnly=yes`.

## "Permission denied" over SSH despite the right key

Most commonly: too many keys in the agent and the remote rejects after N
mismatches. Set `sshKeyPath` to the specific key path. The orchestrator
appends `-o IdentitiesOnly=yes` so only that one key is attempted.

## The Settings → Sync MediaGuard button is missing

Right — the MediaGuard preset was removed in the public release. Your existing
config (projects, processes, deploy scripts) is preserved in
`config.json`, so day-to-day work continues. For one-click preset support of
your own project bundles, watch the
[generic preset framework](https://github.com/blaze-uz/app-orchestrator/issues)
roadmap item or contribute one.

## Where's the config file?

```
~/Library/Application Support/uz.blaze.app-orchestrator/config.json
```

Backups land in the same directory with timestamps. Deploy history is
`deploy-history.jsonl` and recent logs are `log-history.jsonl`.

## How do I reset everything?

Quit the app, then:

```bash
mv "$HOME/Library/Application Support/uz.blaze.app-orchestrator/config.json" \
   "$HOME/Library/Application Support/uz.blaze.app-orchestrator/config.json.before-reset"
```

Reopen the app and it'll write a fresh default config. The old file is right
next to it if you want to inspect or partially restore.

## I want to file a bug

Open an [issue](https://github.com/blaze-uz/app-orchestrator/issues/new/choose).
Include:

- App Orchestrator version (Settings → About).
- macOS version.
- The relevant entry from the deploy history (Settings → Activity for a
  recent timestamped event ID) or a redacted log snippet.
- For HTTP API issues: the exact `curl` invocation with the token redacted.

For security issues, see [SECURITY.md](../SECURITY.md) instead.
