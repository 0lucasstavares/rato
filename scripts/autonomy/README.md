# Autonomy Scripts

GitHub Actions are the primary autonomy runner again. The dashboard now reads workflow runs, PRs, and issues directly from GitHub.

## Primary execution

The live workflows are:

- `.github/workflows/agent-scrum-master.yml`
- `.github/workflows/agent-manager.yml`
- `.github/workflows/agent-worker.yml`
- `.github/workflows/agent-reviewer.yml`
- `.github/workflows/agent-merger.yml`

Use these to toggle scheduled autonomy:

- `.github/workflows/autonomy-on.yml`
- `.github/workflows/autonomy-off.yml`

## Local fallback tools

These scripts remain available for operator fallback and local smoke tests:

- `scripts/autonomy/run-local-autonomy.ps1`: local supervisor.
- `scripts/autonomy/dashboard-server.mjs`: lightweight localhost dashboard server backed by GitHub Actions data.
- `scripts/autonomy/dashboard/index.html`: browser UI backed by GitHub Actions workflow data and run logs.

Run the local supervisor:

```powershell
pwsh ./scripts/autonomy/run-local-autonomy.ps1
```

Run one decision tick only:

```powershell
pwsh ./scripts/autonomy/run-local-autonomy.ps1 -Once
```

Start the dashboard:

```powershell
node ./scripts/autonomy/dashboard-server.mjs
```

Then open:

```text
http://127.0.0.1:19774
```