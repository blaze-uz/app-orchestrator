# Remote machines & SSH

Karvon can drive processes and deploys on remote hosts over SSH. This
page covers what's required and how it works in practice.

## Prerequisites

- The orchestrator runs on macOS or Windows. On Windows it uses the built-in
  OpenSSH client (`ssh.exe`); enable it under *Settings → Apps → Optional
  features* if it is missing.
- The remote host must accept SSH connections. Remote *targets* must be
  macOS/Linux: the orchestrator's remote payload is a POSIX shell script
  (`sh`, `kill`, `$$`), so a Windows remote target is not supported.
- Authentication uses public-key SSH. Passwords and interactive prompts are
  not supported (the client uses `BatchMode=yes`).
- The SSH key must be loaded in the agent, or its path must be set explicitly
  on the machine record (`sshKeyPath`).

That last detail matters: if you have many keys in your agent, the remote may
reject before reaching the right one. Set `sshKeyPath` and the orchestrator
passes `-i` plus `-o IdentitiesOnly=yes`.

## Adding a machine

Sidebar → **Machines** → **+ New machine**:

| Field | Notes |
|---|---|
| Name | A short label, shown in dropdowns. |
| Hostname / IP | DNS name, IP, or Tailscale hostname. |
| SSH user | The remote account. Each host may have a different one. |
| SSH port | Default `22`. |
| SSH key path | Optional. Absolute path on the local Mac. |

Click **Test connection**. The orchestrator runs

```
ssh -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new \
    user@host echo __HERD_PROBE_OK__:$(whoami):$(hostname)
```

and verifies the marker came back.

## Routing work to a machine

Every project, process, and deploy script has a `machineId` field. Set it via
the UI dropdown.

- **Project-level** `machineId` only affects deploy script defaulting — when
  you create new deploy scripts inside that project they inherit it.
- **Process-level** `machineId` determines where the process runs. The
  orchestrator manages its lifecycle (start, stop, health check, restart)
  over SSH.
- **Deploy script-level** `machineId` determines where that single step runs.
  A pipeline can mix local and remote steps freely.

A null `machineId` (or one pointing at the default local machine) means
"local".

## How remote commands are executed

For each remote command the orchestrator builds:

```
ssh \
  -o BatchMode=yes \
  -o ConnectTimeout=10 \
  -o ServerAliveInterval=30 \
  -o ServerAliveCountMax=3 \
  -o StrictHostKeyChecking=accept-new \
  -o LogLevel=ERROR \
  -tt -p <port> \
  [-i <keyPath>] \
  <user>@<host> \
  <generated remote-shell payload>
```

The generated payload:

```sh
printf '__HERD_REMOTE_PID__=%s\n' "$$" 1>&2 \
  && cd '<cwd>' \
  && export KEY='<value>' OTHER='<value>' \
  && exec <user-command>
```

The PID marker lets the orchestrator track and kill the remote process group
on cancel. Single quotes around the cwd and env values are escaped properly,
so values with spaces or special characters are safe.

## Host-key validation

The orchestrator uses `StrictHostKeyChecking=accept-new`. The first connection
to a host adds its key to `~/.ssh/known_hosts`. Subsequent connections require
the same key — a MITM swap will fail loudly.

If you're operating over an actively hostile network, pre-populate
`known_hosts` (e.g. via `ssh-keyscan host >> ~/.ssh/known_hosts`) before
adding the machine to the orchestrator.

## Cancellation

When you cancel a remote command, the orchestrator sends:

```
kill -TERM <remote-pid>
```

over a fresh SSH connection. Only a small allowlist of signals is permitted —
`TERM`, `KILL`, `INT`, `HUP`, `QUIT`, `USR1`, `USR2`, `STOP`, `CONT` — to
avoid signal-name injection through caller data.

## Multi-machine workflows

A common shape is:

- **Local Mac** runs the UI, the API, and the frontend dev server.
- **Mac mini #1** runs AI workers (large memory, separate CPU budget).
- **Mac mini #2** runs queue workers and the scheduler.

You register each Mac as a machine, point each project's processes at the
right one, and drive everything from the local UI.

Logs from remote processes stream back over the same SSH channel and appear in
the local Logs view, indistinguishable from local logs except for the host
label.
