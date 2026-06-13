# Configuration reference

App Orchestrator stores all state as JSON in:

```
~/Library/Application Support/uz.blaze.app-orchestrator/config.json
```

You normally edit it through the UI. This page documents the schema so you can
import/export, write migration scripts, or drive the orchestrator from your
own tools.

## Top-level shape

```jsonc
{
  "configSchemaVersion": 5,
  "workspaces":     [ /* Workspace[] */ ],
  "machines":       [ /* Machine[]   */ ],
  "projects":       [ /* Project[]   */ ],
  "processes":      [ /* ProcessDefinition[] */ ],
  "deployScripts":  [ /* DeployScript[] */ ],
  "activity":       [ /* ActivityEvent[] (capped at 500) */ ],
  "autoDeployState": { /* per-project last-seen git SHA */ },
  "settings":       { /* AppSettings */ },
  "lastSelectedProjectId": "project_…",
  "lastSelectedProcessId": "process_…"
}
```

## Workspace

Top-level grouping. There is exactly one default workspace called `workspace_default`.

```jsonc
{
  "id": "workspace_default",
  "name": "Default",
  "description": null,
  "isDefault": true,
  "createdAt": "2026-04-22T10:29:00Z",
  "updatedAt": "2026-04-22T10:29:00Z"
}
```

## Machine

A machine is either *local* (one default per install) or a remote SSH target.

```jsonc
{
  "id": "machine_…",
  "name": "production",
  "hostname": "prod.example.com",
  "sshUser": "deploy",
  "sshPort": 22,
  "sshKeyPath": "~/.ssh/id_ed25519",   // optional, defaults to SSH agent
  "isDefaultLocal": false,
  "createdAt": "…",
  "updatedAt": "…"
}
```

SSH calls use `StrictHostKeyChecking=accept-new` (TOFU). See [ssh-remote-machines.md](ssh-remote-machines.md).

## Project

```jsonc
{
  "id": "project_…",
  "workspaceId": "workspace_default",
  "name": "MyApp",
  "slug": "myapp",
  "description": "Laravel + Vite + queue worker",
  "rootPath": "/Users/me/Projects/myapp",
  "color": "#31d07f",
  "tags": ["laravel", "web"],
  "autoStart": true,        // start project automatically on app launch
  "startupOrder": 10,       // smaller = earlier
  "memoryLimitMb": null,    // optional, applies to each process under this project
  "autoRestartOnDeploy": true,
  "autoDeploy": true,       // poll git and rerun deploy when remote moves
  "machineId": null,        // null = local; otherwise routes deploy scripts to this machine
  "createdAt": "…",
  "updatedAt": "…"
}
```

## ProcessDefinition

```jsonc
{
  "id": "process_…",
  "projectId": "project_…",
  "name": "Laravel HTTP server",
  "key": "laravel-http",
  "command": "php",
  "args": ["artisan", "serve", "--host=127.0.0.1", "--port=8000"],
  "workingDirectory": null,   // null = use project.rootPath
  "env": { "APP_ENV": "local" },
  "memoryLimitMb": 512,
  "autoStart": true,
  "restartPolicy": {
    "kind": "on-failure",     // "never" | "on-failure" | "always" | "limited-retries"
    "maxRetries": null,
    "retryDelayMs": 3000
  },
  "startupDelayMs": null,
  "dependsOn": ["laravel-http"],    // keys of processes that must be running first
  "healthCheck": {
    "kind": "http",                 // "none" | "tcp" | "http" | "command"
    "url": "http://127.0.0.1:8000/up",
    "method": "GET",
    "expectedStatus": 200,
    "timeoutMs": 2000
  },
  "logMode": "combined",            // "combined" | "split"
  "group": "web",
  "visible": true,
  "machineId": null,                // null = run on local
  "createdAt": "…",
  "updatedAt": "…"
}
```

### Health check kinds

- `none` — no health check.
- `tcp` — open a TCP connection to `host:port` and close immediately.
- `http` — GET (or POST) and assert the response code.
- `command` — run an arbitrary command, treat exit 0 as healthy.

### Restart policies

| kind | Behaviour |
|---|---|
| `never` | If the process exits (success or failure), leave it stopped. |
| `on-failure` | Restart only on non-zero exit. Backoff is exponential up to a cap. |
| `always` | Restart regardless of exit code. |
| `limited-retries` | Restart on failure up to `maxRetries`, then give up. |

`retryDelayMs` is the *base* delay. The orchestrator grows it exponentially and
caps it to keep the restart loop sane.

## DeployScript

```jsonc
{
  "id": "deploy_…",
  "projectId": "project_…",
  "name": "Git pull",
  "stage": "main",                 // "pre" | "main" | "post"
  "order": 0,                      // ordinal within the stage
  "command": "git",
  "args": ["pull", "--ff-only"],
  "workingDirectory": null,        // null = project.rootPath
  "env": {},
  "machineId": null,               // null = run on local; otherwise SSH
  "continueOnError": false,        // if true, pipeline continues on non-zero exit
  "createdAt": "…",
  "updatedAt": "…"
}
```

See [deployments.md](deployments.md) for stage semantics.

## AppSettings

```jsonc
{
  "theme": "dark",                // "dark" | "light"
  "launchOnLogin": false,
  "autoStartMarkedProjects": true,
  "logRetentionLines": 5000,
  "projectStoragePath": null,
  "notificationsEnabled": false,
  "stopTimeoutMs": 5000,
  "httpApiEnabled": false,        // see SECURITY.md before enabling
  "httpApiPort": 8765,
  "httpApiBindHost": "127.0.0.1",
  "httpApiToken": null            // generated on first enable
}
```

## Importing and exporting

- Settings → Config → **Export** writes the full JSON, optionally redacting any
  env value whose key contains `token`, `secret`, `password`, or `key`.
- Settings → Config → **Import** replaces the entire config with the JSON you
  pick. Runtime state (PIDs, log buffers) is reset.

Both operations are also available over the [HTTP API](http-api.md).
