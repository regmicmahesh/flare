# Flare

A Vercel-like deployment platform written in **Rust** (backend) and **React** (frontend).

Link public GitHub repositories (no OAuth / API keys required), detect commits and file changes, run automated builds, and serve preview deployments.

## Architecture

```mermaid
flowchart LR
  UI[React dashboard]
  API[Rust API - Axum]
  DB[(SQLite)]
  Git[git clone / fetch]
  Node[npm build]
  Previews["/_deploy previews"]

  UI -->|/api| API
  UI -->|/_deploy| Previews
  API --> DB
  API --> Git
  Git --> Node
  Node --> Previews
  API --> Previews
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for components, data model, Docker topology, and API notes.

## Features

- **Public GitHub linking** — paste any public repo URL; Flare clones via `git` (no credentials)
- **Auto builds** — poll for new commits (interval configurable in **Settings** / SQLite)
- **Change detection** — commit diffs and changed-file lists per deployment
- **Framework detection** — static sites, Vite, Next.js, Create React App, and more
- **Preview deployments** — unique URL per deployment under `/_deploy/<id>/`
- **Build logs** — streaming / stored logs in the dashboard
- **Projects dashboard** — manage projects, env vars, redeploys
- **Docker Compose** — multi-stage Rust image (with git + node) + nginx frontend
- **Strict CI** — PR checks on `main` / `develop`

## Quick start

### Makefile (recommended)

```bash
make dev-api   # backend on http://127.0.0.1:8080
make dev-ui    # frontend on http://127.0.0.1:5173
make test      # cargo test + frontend build
make lint      # fmt check, clippy -D warnings, frontend lint
```

### Backend

```bash
cd backend
cargo run
# API listens on http://127.0.0.1:8080
```

### Frontend

```bash
cd frontend
npm install
npm run dev
# UI on http://127.0.0.1:5173 (proxies /api and /_deploy to the API)
```

### Docker Compose

```bash
docker compose up --build
# UI:  http://localhost:3000
# API: http://localhost:8080
```

The backend image includes **git** and **node/npm** so it can clone public repos and run npm builds. The frontend image serves the Vite production build via **nginx** and proxies `/api` and `/_deploy` to the backend.

## Link a public repo

In the dashboard: **New Project** → paste `owner/repo` or a full GitHub URL → Flare clones and builds.

### Example public repos (small / static-friendly)

| Repo | Why it’s a good demo |
|------|----------------------|
| [`mdn/beginner-html-site`](https://github.com/mdn/beginner-html-site) | Tiny static HTML site |
| [`mdn/beginner-html-site-styled`](https://github.com/mdn/beginner-html-site-styled) | Static HTML + CSS |
| [`vercel/next.js`](https://github.com/vercel/next.js) | Large Next.js monorepo (heavier build) |
| [`withastro/astro`](https://github.com/withastro/astro) | Astro framework examples |

Prefer small static repos for the fastest first deploy when trying Flare locally.

## Settings

Open **Settings** in the UI (`/settings`) or call:

- `GET /api/settings`
- `PATCH /api/settings` with `{ "poll_interval_secs": 60 }`

Values are stored in the SQLite `settings` table (default poll interval: 60 seconds, minimum 5).

## Branches

| Branch | Purpose |
|--------|---------|
| `main` | Stable releases |
| `develop` | Integration branch |
| `feature/*` | Feature work via PRs |

## License

MIT
