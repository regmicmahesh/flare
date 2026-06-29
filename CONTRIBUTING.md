# Contributing to Flare

## Branch strategy

1. **main** — stable, release-ready only. Merge via PR from `develop` or hotfix PRs.
2. **develop** — integration. All feature work merges here first.
3. **feature/xyz** — short-lived branches off `develop`. Open PRs into `develop`.

## PR workflow

1. `git checkout develop && git pull`
2. `git checkout -b feature/your-thing`
3. Implement + push
4. Open PR → `develop` (use conventional commits in PR title: `feat:`, `fix:`, `chore:`, etc.)
5. Wait for CI
6. Maintainers merge; periodically open `develop` → `main` for stable releases

## Local checks

```bash
cd backend && cargo fmt && cargo clippy -- -D warnings && cargo test
cd frontend && npm ci && npm run build
```
