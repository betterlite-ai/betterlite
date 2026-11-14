import { csvParse } from "d3-dsv";
import { load, loadAll } from "js-yaml";
import { z } from "zod";
import { analyzePath, coverage, mergeTimes, milestoneGroups, PATHS, porting, type Area, type Merge, type Milestone, type PathFacts } from "./backlog.ts";
import { DAY, REPO } from "./format.ts";
import { historyFacts, type History } from "./history/facts.ts";
import {
  agentRunSchema,
  bootSchema,
  configSchema,
  deviationLogSchema,
  entrySchema,
  exceptionSchema,
  hourSchema,
  openPullSchema,
  pullSchema,
  runSchema,
  targetSchema,
  type Config,
  type DeviationLog,
  type Entry,
  type Exception,
  type OpenPull,
} from "./schema.ts";

export interface Site {
  config: Config;
  entries: Entry[];
  byId: Map<string, Entry>;
  milestones: Map<string, Milestone>;
  merges: Merge[];
  counted: Entry[];
  done: number;
  lastWeek: number;
  areas: Area[];
  paths: PathFacts[];
  unmatched: string[];
  logs: DeviationLog[];
  exceptions: Exception[];
  sha: string;
  committed: number;
  history: History;
}

export interface WaitingPull {
  pull: OpenPull;
  age: string;
}

export type Review = { state: "asking" } | { state: "failed"; message: string } | { state: "answered"; pulls: WaitingPull[] };

// A task's pull request comes from a branch named task/<id>.
export function taskOfPull(pull: OpenPull, byId: Map<string, Entry>): Entry | null {
  const task = /^task\/(B\d+)/.exec(pull.head.ref);
  return task === null ? null : (byId.get(task[1]) ?? null);
}

async function fetchText(path: string): Promise<string> {
  const response = await fetch(path, { cache: "no-cache" });
  if (!response.ok) {
    throw new Error(`${path} answered HTTP ${response.status}`);
  }
  return response.text();
}

function checked<T>(path: string, schema: z.ZodType<T>, value: unknown): T {
  const result = schema.safeParse(value);
  if (!result.success) {
    const issue = result.error.issues[0];
    throw new Error(`${path} has an unexpected value at ${issue.path.join(".")}: ${issue.message}`);
  }
  return result.data;
}

// The deploy joins the files with a --- line each, so a file that starts with its own --- leaves an empty document.
function documents(text: string): unknown[] {
  return loadAll(text).filter((document) => document !== null);
}

export async function loadSite(): Promise<Site> {
  const files = ["backlog.yaml", "deviations.yaml", "litellm-inventory.txt", "litellm.yaml", "merges.txt", "commit.txt", "prs.csv", "runs.csv", "agent-runs.csv", "targets.csv", "machine-boot.csv", "machine-hours.csv", "exceptions.csv"];
  const [backlogText, deviationsText, inventoryText, configText, mergesText, commitText, prsText, runsText, agentText, targetsText, bootText, hoursText, exceptionsText] = await Promise.all(files.map(fetchText));
  const entries = checked("backlog.yaml", z.array(entrySchema), documents(backlogText));
  const config = checked("litellm.yaml", configSchema, load(configText));
  const byId = new Map(entries.map((entry) => [entry.id, entry]));
  const byPull = new Map<number, Entry>();
  for (const entry of entries) {
    if (entry.pr !== null) {
      byPull.set(entry.pr, entry);
    }
  }
  const milestones = milestoneGroups(entries);
  const merges = mergeTimes(byPull, mergesText);
  const counted = entries.filter(porting);
  const depths = new Map<string, number>();
  const next = new Map<string, string>();
  const found = new Set(merges.map((merge) => merge.entry.id));
  const [sha, committed] = commitText.trim().split(" ");
  const now = Date.now();
  return {
    config,
    entries,
    byId,
    milestones,
    merges,
    counted,
    done: counted.filter((entry) => entry.status === "done").length,
    lastWeek: merges.filter((merge) => merge.time >= now - 7 * DAY).length,
    areas: coverage(entries, config, inventoryText),
    paths: PATHS.map((path) => analyzePath(path, byId, depths, next)),
    unmatched: entries.filter((entry) => entry.resolution === "merged" && !found.has(entry.id)).map((entry) => entry.id),
    logs: checked("deviations.yaml", z.array(deviationLogSchema), documents(deviationsText)),
    exceptions: checked("exceptions.csv", z.array(exceptionSchema), csvParse(exceptionsText)),
    sha,
    committed: Number(committed) * 1000,
    history: historyFacts(
      {
        pulls: checked("prs.csv", z.array(pullSchema), csvParse(prsText)),
        runs: checked("runs.csv", z.array(runSchema), csvParse(runsText)),
        agentRuns: checked("agent-runs.csv", z.array(agentRunSchema), csvParse(agentText)),
        targets: checked("targets.csv", z.array(targetSchema), csvParse(targetsText)),
        boot: checked("machine-boot.csv", z.array(bootSchema), csvParse(bootText)).at(0) ?? null,
        hours: checked("machine-hours.csv", z.array(hourSchema), csvParse(hoursText)),
        merges,
        milestones,
        byPull,
        entries,
      },
      now,
    ),
  };
}

export async function loadOpenPulls(): Promise<WaitingPull[]> {
  const response = await fetch(`https://api.github.com/repos/${REPO}/pulls?state=open&per_page=100`, { headers: { Accept: "application/vnd.github+json" }, signal: AbortSignal.timeout(10000) });
  if (!response.ok) {
    throw new Error(`GitHub answered HTTP ${response.status}`);
  }
  const body: unknown = await response.json();
  const now = Date.now();
  return checked("GitHub's answer", z.array(openPullSchema), body).map((pull) => {
    const hours = Math.max(0, (now - Date.parse(pull.created_at)) / 3600000);
    return { pull, age: hours < 1 ? "under an hour" : hours < 48 ? `${Math.floor(hours)} h` : `${Math.floor(hours / 24)} days` };
  });
}
