import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..", "..");
const dashboardRoot = path.join(__dirname, "dashboard");
const port = Number(process.env.RATO_AUTONOMY_DASHBOARD_PORT || "19774");

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".js", "application/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
]);

const ghEnv = { ...process.env };
const ghExecutable = process.env.GH_EXECUTABLE || "C:\\Program Files\\GitHub CLI\\gh.exe";
for (const name of [
  "HTTP_PROXY",
  "HTTPS_PROXY",
  "ALL_PROXY",
  "GIT_HTTP_PROXY",
  "GIT_HTTPS_PROXY",
  "CODEX_SANDBOX_NETWORK_DISABLED",
]) {
  delete ghEnv[name];
}

function sendJson(res, status, value) {
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Cache-Control": "no-store",
  });
  res.end(JSON.stringify(value));
}

function runGh(args) {
  const result = spawnSync(ghExecutable, args, {
    cwd: repoRoot,
    env: ghEnv,
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const message = (result.stderr || result.stdout || `gh ${args.join(" ")}`).trim();
    throw new Error(message || `gh ${args.join(" ")} failed`);
  }

  return (result.stdout || "").replace(/^\uFEFF/, "");
}

function parseJson(text, fallback = null) {
  const trimmed = (text || "").trim();
  if (!trimmed) {
    return fallback;
  }
  return JSON.parse(trimmed);
}

function getRepoSlug() {
  const repo = parseJson(runGh(["repo", "view", "--json", "nameWithOwner"]));
  if (!repo?.nameWithOwner) {
    throw new Error("Unable to determine repository slug");
  }
  return repo.nameWithOwner;
}

function toLabelNames(labels) {
  return (labels ?? []).map((label) => (typeof label === "string" ? label : label?.name)).filter(Boolean);
}

function fmtDate(iso) {
  if (!iso) {
    return null;
  }
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? null : date.getTime();
}

function buildQueue(issues, prs) {
  const queue = { ready: 0, working: 0, review: 0, merge: 0, blocked: 0, open_prs: prs.length };
  const readyIssues = [];
  for (const issue of issues) {
    const labels = toLabelNames(issue.labels);
    if (labels.includes("ai:ready")) {
      queue.ready += 1;
      readyIssues.push({ number: issue.number, title: issue.title, labels });
    }
    if (labels.includes("ai:working")) queue.working += 1;
    if (labels.includes("ai:review")) queue.review += 1;
    if (labels.includes("ai:merge")) queue.merge += 1;
    if (labels.includes("ai:blocked")) queue.blocked += 1;
  }
  return { queue, readyIssues };
}

function summarizeRuns(workflows, runs) {
  const byWorkflow = new Map();
  for (const run of runs) {
    const key = run.workflowName || "unknown";
    if (!byWorkflow.has(key)) {
      byWorkflow.set(key, run);
    }
  }

  return workflows.map((workflow) => {
    const latestRun = byWorkflow.get(workflow.name) || null;
    return {
      name: workflow.name,
      path: workflow.path,
      state: workflow.state,
      latest_run: latestRun
        ? {
            id: latestRun.databaseId,
            title: latestRun.displayTitle,
            status: latestRun.status,
            conclusion: latestRun.conclusion,
            created_at: latestRun.createdAt,
            updated_at: latestRun.updatedAt,
            event: latestRun.event,
            head_branch: latestRun.headBranch,
            head_sha: latestRun.headSha,
            url: latestRun.url,
          }
        : null,
    };
  });
}

function getGitHubState() {
  const repoSlug = getRepoSlug();
  const workflows = parseJson(runGh(["workflow", "list", "--repo", repoSlug, "--json", "name,state,path"]), []);
  const runs = parseJson(
    runGh([
      "run",
      "list",
      "--repo",
      repoSlug,
      "--limit",
      "50",
      "--json",
      "databaseId,workflowName,status,conclusion,createdAt,updatedAt,event,headBranch,displayTitle,url,headSha",
    ]),
    [],
  );
  const prs = parseJson(
    runGh([
      "pr",
      "list",
      "--repo",
      repoSlug,
      "--state",
      "open",
      "--limit",
      "50",
      "--json",
      "number,title,isDraft,mergeStateStatus,updatedAt,url,labels,statusCheckRollup",
    ]),
    [],
  );
  const issues = parseJson(
    runGh(["issue", "list", "--repo", repoSlug, "--state", "open", "--limit", "100", "--json", "number,title,labels"]),
    [],
  );
  const variables = parseJson(runGh(["variable", "list", "--repo", repoSlug, "--json", "name,value"]), []);
  const variableMap = Object.fromEntries(variables.map((entry) => [entry.name, entry.value]));
  const { queue, readyIssues } = buildQueue(issues, prs);
  const recentRuns = runs.map((run) => ({
    id: run.databaseId,
    workflow: run.workflowName,
    status: run.status,
    conclusion: run.conclusion,
    started_at: run.createdAt,
    updated_at: run.updatedAt,
    event: run.event,
    branch: run.headBranch,
    sha: run.headSha,
    title: run.displayTitle,
    url: run.url,
  }));
  const workflowsWithRuns = summarizeRuns(workflows, runs);
  const latestFailedRun = recentRuns.find((run) => run.status === "completed" && run.conclusion && run.conclusion !== "success" && run.conclusion !== "skipped") ?? null;

  return {
    repo: { nameWithOwner: repoSlug },
    autonomy: {
      enabled: variableMap.RATO_AUTONOMY ?? "unknown",
      updatedAt: new Date().toISOString(),
    },
    queue,
    workflows: workflowsWithRuns,
    recent_runs: recentRuns,
    pull_requests: prs.map((pr) => ({
      number: pr.number,
      title: pr.title,
      draft: Boolean(pr.isDraft),
      merge_state: pr.mergeStateStatus,
      updated_at: pr.updatedAt,
      url: pr.url,
      labels: toLabelNames(pr.labels),
      checks: Array.isArray(pr.statusCheckRollup) ? pr.statusCheckRollup : [],
    })),
    ready_issues: readyIssues,
    last_error: latestFailedRun
      ? {
          workflow: latestFailedRun.workflow,
          title: latestFailedRun.title,
          conclusion: latestFailedRun.conclusion,
          status: latestFailedRun.status,
          started_at: latestFailedRun.started_at,
          updated_at: latestFailedRun.updated_at,
          url: latestFailedRun.url,
          branch: latestFailedRun.branch,
          sha: latestFailedRun.sha,
        }
      : null,
  };
}

function readRunLog(runId) {
  const slug = getRepoSlug();
  const content = runGh(["run", "view", String(runId), "--repo", slug, "--log"]);
  return content;
}

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "127.0.0.1"}`);
    if (url.pathname === "/api/github-state" || url.pathname === "/api/state") {
      try {
        sendJson(res, 200, getGitHubState());
      } catch (error) {
        sendJson(res, 503, {
          error: "github autonomy state unavailable",
          detail: error instanceof Error ? error.message : String(error),
        });
      }
      return;
    }

    if (url.pathname.startsWith("/api/runs/") && url.pathname.endsWith("/log")) {
      try {
        const id = decodeURIComponent(url.pathname.slice("/api/runs/".length, -"/log".length));
        const content = readRunLog(id);
        sendJson(res, 200, {
          id,
          content,
          updatedAt: new Date().toISOString(),
        });
      } catch (error) {
        sendJson(res, 404, {
          error: "github run log unavailable",
          detail: error instanceof Error ? error.message : String(error),
        });
      }
      return;
    }

    const relativePath = url.pathname === "/" ? "index.html" : url.pathname.replace(/^\/+/, "");
    const target = path.normalize(path.join(dashboardRoot, relativePath));
    if (!target.startsWith(dashboardRoot)) {
      sendJson(res, 400, { error: "invalid path" });
      return;
    }

    const body = await readFile(target);
    const ext = path.extname(target);
    res.writeHead(200, {
      "Content-Type": contentTypes.get(ext) ?? "application/octet-stream",
      "Cache-Control": "no-store",
    });
    res.end(body);
  } catch {
    sendJson(res, 404, { error: "not found" });
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`RATO autonomy dashboard: http://127.0.0.1:${port}`);
});