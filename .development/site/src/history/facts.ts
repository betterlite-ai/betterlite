import { groupOf, type Merge, type Milestone, type PullMerge } from "../backlog.ts";
import { dayKey, idNumber, quantile, sum } from "../format.ts";
import type { AgentRun, Boot, Entry, Hour, Target } from "../schema.ts";

// Stage 2 starts when #722 merges and stage 3 when #1544 does.
export const CUTOVER_PRS = [722, 1544];
export const STAGE_COLORS: Record<number, string> = { 1: "--c7", 2: "--c4", 3: "--c5" };

export type Tally = { [K in keyof AgentRun]: AgentRun[K] extends number | null ? K : never }[keyof AgentRun];

export const WAIT_KINDS: { keys: Tally[]; label: string; color: string }[] = [
  { keys: ["reply_s"], label: "Model replies", color: "--c4" },
  { keys: ["compact_s"], label: "Compaction", color: "--c5" },
  { keys: ["test_s"], label: "Tests", color: "--c1" },
  { keys: ["build_s"], label: "Builds", color: "--c2" },
  { keys: ["lint_s"], label: "Lint and checks", color: "--c3" },
  { keys: ["other_s"], label: "Other commands", color: "--c6" },
  { keys: ["tool_s", "error_s", "rest_s"], label: "Other tools, API errors and the rest", color: "--other" },
];
const WAIT_KEYS = WAIT_KINDS.flatMap((kind) => kind.keys);

export type HourKey = Exclude<keyof Hour, "hour" | "minutes">;

export interface Period {
  key: number;
  label: string;
  from: number;
  to: number;
}

export interface PeriodStats {
  length: number;
  tasks: number;
  tasksADay: number | null;
  agentRuns: number;
  agentHours: number;
  tokensPerTask: number | null;
  heavy: number | null;
  model: number | null;
  speed: number | null;
  target: number | null;
  ssd: number | null;
}

export interface Measured extends Period {
  stats: PeriodStats;
}

export interface Series {
  label: string;
  color: string;
  times: number[];
  weights: number[] | null;
}

export type ViewKey = "tasks" | "milestones";
export type StagedMerge = Merge & { stage: number };
export type StagedAgentRun = AgentRun & { stage: number };
export type StagedHour = Hour & { stage: number };

export interface HistoryData {
  agentRuns: AgentRun[];
  targets: Target[];
  boot: Boot | null;
  hours: Hour[];
  merged: PullMerge[];
  merges: Merge[];
  milestones: Map<string, Milestone>;
  entries: Entry[];
}

interface Staged {
  now: number;
  cutovers: number[];
  tasks: StagedMerge[];
  days: number[];
  agentRuns: StagedAgentRun[];
  traced: StagedAgentRun[];
  hours: StagedHour[];
  targets: Target[];
  boot: Boot | null;
}

export interface History extends Staged {
  live: Measured[];
  columns: Measured[];
  views: Record<ViewKey, Series[]>;
  entries: Entry[];
  milestones: Map<string, Milestone>;
}

function stageOf(time: number, cutovers: number[]): number {
  return 1 + cutovers.filter((cutover) => time > cutover).length;
}

export function inStage(period: Period, stage: number): boolean {
  return period.key === 0 || period.key === stage;
}

export function spanDays(period: Period): number {
  return Math.max(0, (period.to - period.from) / 86400);
}

export function totals(runs: AgentRun[], keys: Tally[]): number {
  return sum(runs.flatMap((run) => keys.map((key) => run[key])));
}

export function weighted(hours: Hour[], key: HourKey): number | null {
  const minutes = sum(hours.map((hour) => hour.minutes));
  return minutes === 0 ? null : sum(hours.map((hour) => hour[key] * hour.minutes)) / minutes;
}

function dayStarts(first: number, now: number): number[] {
  const days: number[] = [];
  const cursor = new Date(first);
  cursor.setHours(0, 0, 0, 0);
  while (cursor.getTime() <= now) {
    days.push(cursor.getTime());
    cursor.setDate(cursor.getDate() + 1);
  }
  return days;
}

// Times before the first day have no slot and are skipped.
export function countPerDay(days: number[], times: number[], weights: number[] | null): number[] {
  const slots = new Map(days.map((day, i) => [dayKey(day), i]));
  const counts = days.map(() => 0);
  times.forEach((time, i) => {
    const slot = slots.get(dayKey(time));
    if (slot !== undefined) {
      counts[slot] += weights === null ? 1 : weights[i];
    }
  });
  return counts;
}

// Each view is a list of series: a label, a color variable and the merge times the series counts.
function historyViews(merges: Merge[], milestones: Map<string, Milestone>): Record<ViewKey, Series[]> {
  const taskTimes = merges.map((merge) => merge.time);
  const byGroup = new Map<Milestone, number[]>();
  for (const merge of merges) {
    const group = groupOf(milestones, merge.entry);
    byGroup.set(group, [...(byGroup.get(group) ?? []), merge.time]);
  }
  // Seven colors plus Other, so the milestones with the fewest merges share Other.
  const ranked = [...byGroup.entries()].sort((a, b) => b[1].length - a[1].length || idNumber(a[0].code) - idNumber(b[0].code));
  const colored = ranked.slice(0, 7).sort((a, b) => idNumber(a[0].code) - idNumber(b[0].code));
  const rest = ranked.slice(7).sort((a, b) => idNumber(a[0].code) - idNumber(b[0].code));
  const byMilestone = colored.map(([group, times], i) => ({ label: group.name, color: `--c${i + 1}`, times, weights: null }));
  if (rest.length > 0) {
    byMilestone.push({ label: `Other: ${rest.map(([group]) => group.code).join(", ")}`, color: "--other", times: rest.flatMap(([, times]) => times), weights: null });
  }
  return {
    tasks: [{ label: "Tasks", color: "--done", times: taskTimes, weights: null }],
    milestones: byMilestone,
  };
}

// The machine's own average since it started counts for stage 1, then the 5-minute samples go by their hour.
function ssdPerDay(facts: Staged, period: Period): number | null {
  let written = 0;
  let minutes = 0;
  if (facts.boot !== null && period.key <= 1) {
    written += facts.boot.written_tb * 1000;
    minutes += (facts.boot.to - facts.boot.from) / 60;
  }
  for (const hour of facts.hours.filter((row) => inStage(period, row.stage))) {
    written += (hour.written_gb_h * hour.minutes) / 60;
    minutes += hour.minutes;
  }
  return minutes === 0 ? null : (written / minutes) * 1440;
}

function periodStats(facts: Staged, period: Period): PeriodStats {
  const length = spanDays(period);
  const tasks = facts.tasks.filter((merge) => inStage(period, merge.stage)).length;
  const agents = facts.agentRuns.filter((run) => inStage(period, run.stage));
  const workers = facts.traced.filter((run) => inStage(period, run.stage));
  const active = totals(workers, WAIT_KEYS);
  const replies = totals(workers, ["reply_s"]);
  const disk = facts.targets.filter((target) => inStage(period, target.stage)).map((target) => target.gb);
  return {
    length,
    tasks,
    tasksADay: length > 0 ? tasks / length : null,
    agentRuns: agents.length,
    agentHours: sum(agents.map((run) => (run.end - run.start) / 3600)),
    tokensPerTask: tasks === 0 ? null : sum(agents.map((run) => run.output_tokens)) / tasks,
    heavy: active === 0 ? null : totals(workers, ["test_s", "build_s"]) / active,
    model: active === 0 ? null : totals(workers, ["reply_s", "compact_s", "error_s"]) / active,
    speed: replies === 0 ? null : totals(workers, ["output_tokens"]) / replies,
    target: quantile(disk, 0.5),
    ssd: ssdPerDay(facts, period),
  };
}

export function historyFacts(data: HistoryData, clock: number): History {
  const now = clock / 1000;
  const cutovers = CUTOVER_PRS.map((number) => data.merged.find((merge) => merge.pull === number)?.time ?? null).filter((time) => time !== null);
  const tasks = data.merges.map((merge) => ({ ...merge, stage: stageOf(merge.time / 1000, cutovers) }));
  const days = data.merged.length === 0 ? [] : dayStarts(Math.min(...data.merged.map((merge) => merge.time)) * 1000, clock);
  const first = days.length === 0 ? now : days[0] / 1000;
  const agentRuns = data.agentRuns.map((run) => ({ ...run, model: run.model.replace(/-\d{8}$/, ""), stage: stageOf(run.start, cutovers) }));
  const staged: Staged = {
    now,
    cutovers,
    tasks,
    days,
    agentRuns,
    traced: agentRuns.filter((run) => run.role === "w" && run.calls !== null),
    hours: data.hours.map((hour) => ({ ...hour, stage: stageOf(hour.hour, cutovers) })),
    targets: data.targets,
    boot: data.boot,
  };
  const stages = [first, ...cutovers].map((from, i) => ({ key: i + 1, label: `Stage ${i + 1}`, from, to: cutovers.at(i) ?? now }));
  const live = stages.map((period) => ({ ...period, stats: periodStats(staged, period) }));
  const all = { key: 0, label: "All", from: first, to: now };
  return {
    ...staged,
    live,
    columns: cutovers.length === 0 ? live : [...live, { ...all, stats: periodStats(staged, all) }],
    views: historyViews(data.merges, data.milestones),
    entries: data.entries,
    milestones: data.milestones,
  };
}
