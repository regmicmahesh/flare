# Flare CLI recipes (`curl`)

Flare has no dedicated CLI binary yet. Use **`curl`** against the HTTP API (default `http://127.0.0.1:8080`). Set `BASE` to your instance URL.

```bash
export BASE=http://127.0.0.1:8080
```

Public GitHub only — no OAuth, no API keys. Protect secrets yourself (never pipe env **values** into export/import).

---

## Health & version

```bash
curl -sS "$BASE/api/health" | jq .
curl -sS "$BASE/api/version" | jq .
```

---

## List & search projects

```bash
# All projects
curl -sS "$BASE/api/projects" | jq .

# Search by name, slug, or owner/repo
curl -sS "$BASE/api/projects?q=mdn" | jq .
```

---

## Link a public repo (create project)

```bash
curl -sS -X POST "$BASE/api/projects" \
  -H 'Content-Type: application/json' \
  -d '{
    "github": "mdn/beginner-html-site",
    "name": "MDN demo",
    "branch": "main"
  }' | jq .

# owner/repo or full URL both work:
# "github": "https://github.com/mdn/beginner-html-site"
```

Response includes `id` — use it in the recipes below (`PROJECT_ID`).

```bash
export PROJECT_ID='<project-uuid>'
```

---

## Get / update / delete project

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID" | jq .

curl -sS -X PATCH "$BASE/api/projects/$PROJECT_ID" \
  -H 'Content-Type: application/json' \
  -d '{
    "build_command": "npm run build",
    "output_directory": "dist",
    "ignore_patterns": "*.md\ndocs/**",
    "poll_enabled": true,
    "redeploy_interval_mins": 0
  }' | jq .

curl -sS -X DELETE "$BASE/api/projects/$PROJECT_ID"
# 204 No Content
```

---

## Deploy (manual / specific commit)

```bash
# Deploy branch HEAD
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/deploy" \
  -H 'Content-Type: application/json' \
  -d '{}' | jq .

# Deploy a specific commit
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/deploy" \
  -H 'Content-Type: application/json' \
  -d '{"commit_sha": "abc123…"}' | jq .
```

List recent commits:

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/commits?limit=20" | jq .
```

---

## Deployments, logs, cancel

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/deployments" | jq .

export DEPLOY_ID='<deployment-uuid>'

curl -sS "$BASE/api/deployments/$DEPLOY_ID" | jq .
curl -sS "$BASE/api/deployments/$DEPLOY_ID/logs" | jq .

# Cancel queued or building (best-effort)
curl -sS -X POST "$BASE/api/deployments/$DEPLOY_ID/cancel"
```

Preview URL (when `status` is `ready`): `url_path` on the deployment, typically `/_deploy/<id>/`.

---

## Promote & rollback (production pin)

```bash
# Pin a ready deployment as production (used by /s/{slug}/ and custom domains)
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/promote" \
  -H 'Content-Type: application/json' \
  -d "{\"deployment_id\": \"$DEPLOY_ID\"}" | jq .

# Instant rollback to previous ready deployment (or pass deployment_id)
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/rollback" \
  -H 'Content-Type: application/json' \
  -d '{}' | jq .

curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/rollback" \
  -H 'Content-Type: application/json' \
  -d "{\"deployment_id\": \"$DEPLOY_ID\"}" | jq .
```

Production aliases:

- `/s/{slug}/` — slug from project
- `/p/{project_id}/` — by project id

---

## Export project (redacted JSON)

Exports **settings**, **env keys only** (not values), **domain hosts**, **webhook URLs**, and **ignore patterns**. Does **not** include env values, protection passwords, or other secrets.

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/export" | jq .

# Save for backup / migration
curl -sS "$BASE/api/projects/$PROJECT_ID/export" -o flare-project-export.json
```

Example shape:

```json
{
  "version": 1,
  "name": "MDN demo",
  "github_url": "https://github.com/mdn/beginner-html-site",
  "owner_repo": "mdn/beginner-html-site",
  "default_branch": "main",
  "framework": "static",
  "root_directory": ".",
  "build_command": null,
  "output_directory": null,
  "install_command": null,
  "ignore_patterns": "*.md\ndocs/**",
  "poll_enabled": true,
  "redeploy_interval_mins": 0,
  "password_protect": false,
  "env_keys": ["NODE_ENV", "PUBLIC_API_URL"],
  "domain_hosts": ["app.local"],
  "webhooks": [{ "url": "https://example.com/hooks/flare", "events": "deployment.ready,deployment.error,deployment.queued" }]
}
```

---

## Import project (from export overrides, no secrets)

`POST /api/projects/import` requires `github` and accepts optional non-secret overrides (typically from an export). **Env values and passwords are never accepted** — set them after import.

```bash
# Minimal
curl -sS -X POST "$BASE/api/projects/import" \
  -H 'Content-Type: application/json' \
  -d '{"github": "mdn/beginner-html-site"}' | jq .

# From export fields (jq builds a safe body — no secrets)
curl -sS -X POST "$BASE/api/projects/import" \
  -H 'Content-Type: application/json' \
  -d "$(jq '{
    github: .owner_repo,
    name: .name,
    branch: .default_branch,
    root_directory: .root_directory,
    build_command: .build_command,
    output_directory: .output_directory,
    install_command: .install_command,
    ignore_patterns: .ignore_patterns,
    poll_enabled: .poll_enabled,
    redeploy_interval_mins: .redeploy_interval_mins,
    domain_hosts: .domain_hosts,
    webhooks: [.webhooks[] | {url, events}],
    env_keys: .env_keys
  }' flare-project-export.json)" | jq .
```

After import, restore env **values** yourself (they were never in the export):

```bash
export NEW_ID='<new-project-uuid>'
curl -sS -X POST "$BASE/api/projects/$NEW_ID/env" \
  -H 'Content-Type: application/json' \
  -d '{"key": "NODE_ENV", "value": "production"}' | jq .
```

---

## Environment variables

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/env" | jq .

curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/env" \
  -H 'Content-Type: application/json' \
  -d '{"key": "API_URL", "value": "https://example.com"}' | jq .

curl -sS -X DELETE "$BASE/api/projects/$PROJECT_ID/env/API_URL"
```

---

## Webhooks (deploy hooks)

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/webhooks" | jq .

curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/webhooks" \
  -H 'Content-Type: application/json' \
  -d '{"url": "https://example.com/hooks/flare"}' | jq .

curl -sS -X DELETE "$BASE/api/projects/$PROJECT_ID/webhooks/$WEBHOOK_ID"
```

Events: `deployment.queued`, `deployment.ready`, `deployment.error`.

---

## Custom domains

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/domains" | jq .

curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/domains" \
  -H 'Content-Type: application/json' \
  -d '{"host": "app.local"}' | jq .

curl -sS -X DELETE "$BASE/api/projects/$PROJECT_ID/domains/$DOMAIN_ID"
```

Point DNS or `/etc/hosts` at the Flare host; requests whose `Host` matches serve the production / latest ready deployment.

---

## Password protection

```bash
# Enable
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/protection" \
  -H 'Content-Type: application/json' \
  -d '{"password": "s3cret"}' | jq .

# Clear
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/protection" \
  -H 'Content-Type: application/json' \
  -d '{"password": null}' | jq .
```

Protected previews accept `Authorization: Bearer <password|token>` or cookie `flare_access={project_id}:{token}`.

---

## Platform settings

```bash
curl -sS "$BASE/api/settings" | jq .

curl -sS -X PATCH "$BASE/api/settings" \
  -H 'Content-Type: application/json' \
  -d '{"poll_interval_secs": 60}' | jq .
```

---

## Activity & analytics (optional)

```bash
curl -sS "$BASE/api/projects/$PROJECT_ID/activity" | jq .
curl -sS "$BASE/api/projects/$PROJECT_ID/stats" | jq .
curl -sS "$BASE/api/deployments/$DEPLOY_ID/stats" | jq .
curl -sS "$BASE/api/deployments/$DEPLOY_A/diff/$DEPLOY_B" | jq .
```

---

## End-to-end sketch

```bash
export BASE=http://127.0.0.1:8080

# 1. Link public repo
PROJECT_ID=$(curl -sS -X POST "$BASE/api/projects" \
  -H 'Content-Type: application/json' \
  -d '{"github":"mdn/beginner-html-site"}' | jq -r .id)

# 2. Deploy HEAD
DEPLOY_ID=$(curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/deploy" \
  -H 'Content-Type: application/json' -d '{}' | jq -r .id)

# 3. Poll logs until ready
curl -sS "$BASE/api/deployments/$DEPLOY_ID/logs" | jq .

# 4. Promote when ready
curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/promote" \
  -H 'Content-Type: application/json' \
  -d "{\"deployment_id\":\"$DEPLOY_ID\"}" | jq .

# 5. Export (redacted)
curl -sS "$BASE/api/projects/$PROJECT_ID/export" -o export.json
```

See also [ARCHITECTURE.md](./ARCHITECTURE.md) and the root [README.md](../README.md).
