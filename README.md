# Flare

A Vercel-like deployment platform written in **Rust** (backend) and **React** (frontend).

Link public GitHub repositories (no OAuth / API keys required), detect commits and file changes, run automated builds, and serve preview deployments.

## Features

- **Public GitHub linking** — paste any public repo URL; Flare clones via `git` (no credentials)
- **Auto builds** — poll for new commits and trigger builds
- **Change detection** — commit diffs and changed-file lists per deployment
- **Framework detection** — static sites, Vite, Next.js, Create React App, and more
- **Preview deployments** — unique URL per deployment
- **Build logs** — streaming / stored logs in the dashboard
- **Projects dashboard** — manage projects, env vars, redeploys
- **Strict CI** — PR checks on `main` / `develop`

## Quick start

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
# UI on http://127.0.0.1:5173
```

### Link a public repo

In the dashboard: **New Project** → paste e.g. `https://github.com/vercel/next.js` or `owner/repo` → Flare clones and builds.

## Branches

| Branch | Purpose |
|--------|---------|
| `main` | Stable releases |
| `develop` | Integration branch |
| `feature/*` | Feature work via PRs |

## License

MIT
