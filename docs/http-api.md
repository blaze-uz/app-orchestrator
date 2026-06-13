# HTTP API

App Orchestrator ships an optional HTTP API for scripting, automation, and
integration with monitoring tools.

> ⚠️ Read [SECURITY.md](../SECURITY.md) before exposing the API. Every protected
> endpoint can spawn processes, run deploy scripts, and execute commands on
> registered remote machines.

## Enabling the API

The API is **disabled by default**. Enable it in **Settings → Config → HTTP API**:

| Field | Default | Notes |
|---|---|---|
| Enabled | `false` | Off until you flip it. |
| Bind host | `127.0.0.1` | Set to a LAN IP (or `0.0.0.0`) to expose. |
| Port | `8765` | Pick anything unused. |
| Token | _auto-generated_ | 32 random bytes, hex. Persisted in `config.json`. |

When enabled, the orchestrator logs:

```
[http-api] listening on 127.0.0.1:8765 (token: a1b2c3…)
```

Only the first 6 hex characters of the token are printed. Read the full token
from the Settings panel or the config file.

## Authentication

Every protected endpoint requires `Authorization: Bearer <token>`. The public
endpoints (no auth) are limited to `/api/v1/health`.

```bash
TOKEN="paste-your-token-here"
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8765/api/v1/dashboard
```

A request without the header or with the wrong token returns `401`.

## CORS

The API responds with permissive CORS headers but does *not* allow credentials.
Browser pages cannot send `Authorization` headers cross-origin. CLI clients
(curl, scripts, server-side fetches) are unaffected.

## Endpoint reference

All endpoints are versioned under `/api/v1`.

### Public

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/health` | Liveness probe. Returns `{ok, name, version}`. |

### Config & settings

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/config` | Return the full `AppConfig`. |
| `POST` | `/api/v1/config/import` | Replace the full config. |
| `GET` | `/api/v1/config/export?redactSecrets=true` | Return the config as a pretty-printed JSON string. |
| `PUT` | `/api/v1/settings` | Replace `AppSettings` only. |

### Workspaces

| Method | Path |
|---|---|
| `GET` | `/api/v1/workspaces` |
| `POST` | `/api/v1/workspaces` |
| `PATCH` | `/api/v1/workspaces/:id` |
| `DELETE` | `/api/v1/workspaces/:id` |

### Machines

| Method | Path |
|---|---|
| `GET` | `/api/v1/machines` |
| `POST` | `/api/v1/machines` |
| `PATCH` | `/api/v1/machines/:id` |
| `DELETE` | `/api/v1/machines/:id` |
| `POST` | `/api/v1/machines/:id/test` |

### Projects

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/projects` | List. |
| `POST` | `/api/v1/projects` | Create. |
| `GET` | `/api/v1/projects/:id` | Detail view including processes. |
| `PATCH` | `/api/v1/projects/:id` | Update. |
| `DELETE` | `/api/v1/projects/:id` | Delete (cascades to processes & deploys). |
| `POST` | `/api/v1/projects/:id/start` | Start every process. |
| `POST` | `/api/v1/projects/:id/start-auto` | Start only processes marked `autoStart`. |
| `POST` | `/api/v1/projects/:id/stop` | Stop every running process. |
| `POST` | `/api/v1/projects/:id/restart` | Restart all running processes. |
| `GET` | `/api/v1/projects/:id/processes` | List process definitions for this project. |
| `POST` | `/api/v1/projects/:id/validate-path` | Sanity-check a root-path string. |

### Process definitions

| Method | Path |
|---|---|
| `POST` | `/api/v1/process-definitions` |
| `PATCH` | `/api/v1/process-definitions/:id` |
| `DELETE` | `/api/v1/process-definitions/:id` |

### Process runtime

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/processes` | Runtime state for every process. |
| `GET` | `/api/v1/processes/:id` | One process's runtime state. |
| `GET` | `/api/v1/processes/:id/metrics` | Recent metric samples (CPU, RSS). |
| `POST` | `/api/v1/processes/:id/start` | |
| `POST` | `/api/v1/processes/:id/stop` | |
| `POST` | `/api/v1/processes/:id/restart` | |
| `POST` | `/api/v1/processes/:id/health-check` | Run the health check now. |
| `POST` | `/api/v1/processes/restart-failed` | Restart every failed process (optional `?projectId=`). |

### Logs

| Method | Path | Query |
|---|---|---|
| `GET` | `/api/v1/logs` | `projectId`, `processId`, `limit`, `since` |
| `DELETE` | `/api/v1/logs` | `projectId` (optional) |
| `GET` | `/api/v1/logs/export` | Same as above, returns a JSON string. |

### Deploys

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/projects/:id/deploy-scripts` | List. |
| `POST` | `/api/v1/deploy-scripts` | Create. |
| `PATCH` | `/api/v1/deploy-scripts/:id` | Update. |
| `DELETE` | `/api/v1/deploy-scripts/:id` | Delete. |
| `POST` | `/api/v1/projects/:id/deploy-scripts/reorder` | Body: `{orderedIds: string[]}`. |
| `GET` | `/api/v1/projects/:id/deploys` | Historical runs for this project. |
| `POST` | `/api/v1/projects/:id/deploy` | Start a deploy. |
| `POST` | `/api/v1/projects/:id/cancel-deploy` | Cancel the running deploy. |
| `GET` | `/api/v1/projects/:id/deploy-state` | Current run state. |
| `GET` | `/api/v1/deploys` | All in-memory run states. |
| `GET` | `/api/v1/deploys/:runId` | One historical run with logs. |

### Observability

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/health-summary` | Roll-up counts (`?projectId=` optional). |
| `GET` | `/api/v1/dashboard` | Same data the Dashboard view uses. |
| `GET` | `/api/v1/activity` | Recent activity events (`?limit=50`). |

### External processes

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/v1/projects/:id/external-processes` | Processes started outside the orchestrator that the orchestrator detected. |
| `POST` | `/api/v1/external-processes/:gid/stop` | Stop one. |
| `GET` | `/api/v1/ports` | Detected port bindings. |
| `GET` | `/api/v1/ports/:port` | Find the process holding a port. |

## Response shape

Successful responses:

```json
{ "success": true, "data": { /* … */ } }
```

Errors:

```json
{
  "success": false,
  "error": {
    "code": "VALIDATION_FAILED",
    "message": "Project name is required",
    "retryable": false
  }
}
```

HTTP status codes track the error code: `NOT_FOUND` → 404, `INVALID` /
`VALIDATION` / `LOCKED` / `IN_USE` → 400, anything else → 500.

## Worked example

Spin up a project from a script:

```bash
TOKEN="…"
BASE="http://127.0.0.1:8765/api/v1"

PROJECT_ID=$(curl -s -X POST "$BASE/projects" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"My App","rootPath":"/Users/me/Projects/my-app","autoStart":true,"startupOrder":10,"tags":["api"]}' \
  | jq -r '.data.id')

curl -s -X POST "$BASE/process-definitions" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"projectId\":\"$PROJECT_ID\",\"name\":\"API\",\"key\":\"api\",\"command\":\"php\",\"args\":[\"artisan\",\"serve\"],\"env\":{},\"autoStart\":true,\"restartPolicy\":{\"kind\":\"on-failure\",\"retryDelayMs\":3000},\"dependsOn\":[],\"healthCheck\":{\"kind\":\"none\"},\"logMode\":\"combined\",\"visible\":true}"

curl -s -X POST "$BASE/projects/$PROJECT_ID/start" \
  -H "Authorization: Bearer $TOKEN"
```
