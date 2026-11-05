import { GITHUB, DAY, number, oneDecimal, compact, shortDate, yearDate, longDate, dayTime, hourOnly, esc, idNumber, figure, dayKey, cssColor, duration, quantile, plural } from "./shared.js";

const CUTOVER_PR = 722;
const MEDIAN_WINDOW = 20;
// Gridlines of the logarithmic time scales, in minutes.
const STAGE_TICKS = [1, 5, 15, 60, 240, 1440, 4320, 10080];
// Steps between date ticks, in seconds: whole days on the daily charts, hours too on the others.
const DAY_STEPS = [86400, 172800, 604800, 1209600, 2419200];
const HOUR_STEPS = [3600, 10800, 21600, 43200, ...DAY_STEPS];
const STEPS = [
  { key: "build", label: "Build", about: "First commit to pull request opened, including the local checks, review and fix rounds, usage-limit pauses and the wait for the architect to push", color: "--c1" },
  { key: "checks", label: "Checks", about: "Opened to added to the merge queue", color: "--c2" },
  { key: "queue", label: "Queue", about: "Added to the merge queue to merged", color: "--c3" },
];
const TOTAL = { key: "total", label: "Total", about: "First commit to merged" };
const STAGE_COLORS = { 1: "--c7", 2: "--c4" };
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const SIZE_BUCKETS = [
  { label: "Under 100", from: 0, to: 100 },
  { label: "100 to 499", from: 100, to: 500 },
  { label: "500 to 1,999", from: 500, to: 2000 },
  { label: "2,000 to 4,999", from: 2000, to: 5000 },
  { label: "5,000 and up", from: 5000, to: Infinity },
];
const CI_EVENTS = [
  { key: "pull_request", label: "Pull request", color: "--c1" },
  { key: "merge_group", label: "Merge queue", color: "--c2" },
  { key: "push", label: "Push on main", color: "--c3" },
];
const CI_RESULTS = [
  { key: "success", label: "Passed", color: "--c3" },
  { key: "failure", label: "Failed", color: "--c2" },
  { key: "cancelled", label: "Cancelled", color: "--other" },
];
const SHELL_KINDS = [
  { key: "test_s", label: "Tests", color: "--c1" },
  { key: "build_s", label: "Builds", color: "--c2" },
  { key: "lint_s", label: "Lint and checks", color: "--c3" },
  { key: "other_s", label: "Other", color: "--other" },
];
const VIOLIN = { lane: 34, label: 64, axis: 18, points: 80, gap: 44 };
const VIOLIN_ABOUT = {
  time: "Time per task pull request in each step. A step under a minute counts as one minute.",
  size: "Lines changed, files changed and commits per task pull request.",
  ci: "Wall time of the gate runs that passed or failed, by event. Cancelled runs are left out, and a run under a minute counts as one minute.",
  agents: "Wall time of each worker and reviewer run, from launch to exit, and the output tokens of each worker run.",
  disk: "Size of a task checkout's build folder when it was deleted after the task merged.",
};
const MACHINE_ROWS = [
  { label: "SSD written", unit: "GB an hour", key: "written_gb_h", boot: (boot) => (boot.written_tb * 1000) / bootHours(boot) },
  { label: "SSD read", unit: "GB an hour", key: "read_gb_h", boot: (boot) => (boot.read_tb * 1000) / bootHours(boot) },
  { label: "CPU busy", unit: "%", key: "cpu_busy", boot: (boot) => boot.cpu_busy },
  { label: "CPU waiting on disk", unit: "%", key: "iowait", boot: (boot) => boot.iowait },
  { label: "Disk stall", unit: "% of time", key: "io_stall", boot: (boot) => boot.io_stall },
  { label: "Memory stall", unit: "% of time", key: null, boot: (boot) => boot.memory_stall },
  { label: "RAM in use", unit: "%", key: "memory_used", boot: null },
  { label: "Swap in use", unit: "GB", key: "swap_gb", boot: null },
  { label: "Disk full", unit: "%", key: "disk_used", boot: null },
  { label: "Agent flows running", unit: "", key: "flows", boot: null },
  { label: "rustc processes", unit: "", key: "rustc", boot: null },
];
const MACHINE_CHARTS = [
  { title: "SSD written", unit: "GB an hour", key: "written_gb_h" },
  { title: "CPU busy", unit: "%", key: "cpu_busy" },
  { title: "Disk stall", unit: "% of time", key: "io_stall" },
  { title: "RAM in use", unit: "%", key: "memory_used" },
  { title: "rustc processes", unit: "average", key: "rustc" },
  { title: "Agent flows running", unit: "average", key: "flows" },
];

function bootHours(boot) {
  return (boot.to - boot.from) / 3600;
}

function stageOf(time, cutover) {
  return cutover === null || time <= cutover ? 1 : 2;
}

function inStage(period, stage) {
  return period.key === 0 || period.key === stage;
}

function spanDays(period) {
  return Math.max(0, (period.to - period.from) / 86400);
}

function sum(values) {
  return values.reduce((total, value) => total + (value ?? 0), 0);
}

function mean(values) {
  const known = values.filter((value) => value !== null);
  return known.length === 0 ? null : sum(known) / known.length;
}

function weighted(hours, key) {
  const minutes = sum(hours.map((hour) => hour.minutes));
  return minutes === 0 ? null : sum(hours.map((hour) => hour[key] * hour.minutes)) / minutes;
}

function minutesBetween(from, to) {
  return from !== null && to !== null ? Math.max(0, (to - from) / 60) : null;
}

function pressed(name) {
  return document.querySelector(`.modes[data-chart="${name}"] [aria-pressed="true"]`).value;
}

function dayStarts(first) {
  const days = [];
  const cursor = new Date(first);
  cursor.setHours(0, 0, 0, 0);
  while (cursor.getTime() <= Date.now()) {
    days.push(cursor.getTime());
    cursor.setDate(cursor.getDate() + 1);
  }
  return days;
}

// Times before the first day have no slot and are skipped.
function countPerDay(days, times, weights) {
  const slots = new Map(days.map((day, i) => [dayKey(day), i]));
  const counts = days.map(() => 0);
  times.forEach((time, i) => {
    const slot = slots.get(dayKey(time));
    if (slot !== undefined) {
      counts[slot] += weights ? weights[i] : 1;
    }
  });
  return counts;
}

// Pull request runs often carry no PR number, so they are matched by branch within the PR's lifetime.
function attachRuns(runs, pulls) {
  const byBranch = new Map();
  for (const pull of pulls) {
    byBranch.set(pull.branch, [...(byBranch.get(pull.branch) ?? []), pull]);
  }
  const counts = new Map();
  for (const run of runs) {
    let pr = run.pr;
    if (pr === null && run.event === "pull_request") {
      pr = (byBranch.get(run.branch) ?? []).find((pull) => run.created >= pull.first - 300 && run.created <= pull.merged + 300)?.number ?? null;
    }
    if (pr !== null && run.event !== "push") {
      counts.set(pr, (counts.get(pr) ?? 0) + 1);
    }
  }
  return counts;
}

function stageRows(pulls, byPull, cutover, runCounts) {
  const rows = pulls.map((pull) => ({
    number: pull.number,
    task: byPull.get(pull.number)?.id,
    merged: pull.merged,
    x: pull.merged,
    mergedStage: stageOf(pull.merged, cutover),
    stage: cutover === null || pull.merged <= cutover ? 1 : pull.first > cutover ? 2 : 0,
    build: minutesBetween(pull.first, pull.opened),
    checks: minutesBetween(pull.opened, pull.queued),
    queue: minutesBetween(pull.queued, pull.merged),
    total: minutesBetween(pull.first, pull.merged),
    lines: pull.lines,
    files: pull.files,
    commits: pull.commits,
    queues: pull.queues,
    runs: runCounts.get(pull.number) ?? 0,
  })).sort((a, b) => a.merged - b.merged);
  // uPlot needs rising x values, and some pull requests merged in the same second.
  for (let i = 1; i < rows.length; i++) {
    rows[i].x = Math.max(rows[i].x, rows[i - 1].x + 1);
  }
  return rows;
}

// Each view is a list of series: a label, a color variable and the merge times the series counts.
function historyViews(merges, milestones, pulls, byPull) {
  const taskTimes = merges.map((merge) => merge.time);
  const byCode = new Map();
  for (const merge of merges) {
    const code = merge.entry.milestone.split(" ")[0];
    byCode.set(code, [...(byCode.get(code) ?? []), merge.time]);
  }
  // Seven colors plus Other, so the milestones with the fewest merges share Other.
  const ranked = [...byCode.keys()].sort((a, b) => byCode.get(b).length - byCode.get(a).length || idNumber(a) - idNumber(b));
  const colored = ranked.slice(0, 7).sort((a, b) => idNumber(a) - idNumber(b));
  const rest = ranked.slice(7).sort((a, b) => idNumber(a) - idNumber(b));
  const byMilestone = colored.map((code, i) => ({ label: milestones.get(code).name, color: `--c${i + 1}`, times: byCode.get(code) }));
  if (rest.length > 0) {
    byMilestone.push({ label: `Other: ${rest.join(", ")}`, color: "--other", times: rest.flatMap((code) => byCode.get(code)) });
  }
  const otherTimes = pulls.filter((pull) => !byPull.has(pull.number)).map((pull) => pull.merged * 1000);
  return {
    tasks: [{ label: "Tasks", color: "--done", times: taskTimes }],
    milestones: byMilestone,
    prs: [{ label: "Tasks", color: "--c6", times: taskTimes }, { label: "Other PRs", color: "--c1", times: otherTimes }],
  };
}

function historyFacts(data) {
  const now = Date.now() / 1000;
  const cutover = data.pulls.find((pull) => pull.number === CUTOVER_PR)?.merged ?? null;
  const rows = stageRows(data.pulls, data.byPull, cutover, attachRuns(data.runs, data.pulls));
  const tasks = data.merges.map((merge) => ({ ...merge, stage: stageOf(merge.time / 1000, cutover) }));
  const days = rows.length === 0 ? [] : dayStarts(Math.min(rows[0].merged * 1000, ...tasks.map((merge) => merge.time)));
  const first = days.length === 0 ? now : days[0] / 1000;
  const periods = [
    { key: 1, label: "Stage 1", from: first, to: cutover ?? now, empty: false },
    { key: 2, label: "Stage 2", from: cutover ?? now, to: now, empty: cutover === null },
    { key: 0, label: "All", from: first, to: now, empty: false },
  ];
  return {
    ...data,
    now,
    cutover,
    rows,
    taskRows: rows.filter((row) => row.task),
    tasks,
    days,
    periods,
    live: periods.filter((period) => period.key !== 0 && !period.empty),
    columns: cutover === null ? periods.slice(0, 1) : periods,
    runs: data.runs.map((run) => ({ ...run, stage: stageOf(run.created, cutover) })),
    agentRuns: data.agentRuns.map((run) => ({ ...run, model: String(run.model).replace(/-\d{8}$/, ""), stage: stageOf(run.start, cutover) })),
    hours: data.hours.map((hour) => ({ ...hour, stage: stageOf(hour.hour, cutover) })),
    views: historyViews(data.merges, data.milestones, data.pulls, data.byPull),
  };
}

// The machine's own average since it started counts for stage 1, then the 5-minute samples go by their hour.
function ssdPerDay(facts, period) {
  let written = 0;
  let minutes = 0;
  if (facts.boot && period.key !== 2) {
    written += facts.boot.written_tb * 1000;
    minutes += (facts.boot.to - facts.boot.from) / 60;
  }
  for (const hour of facts.hours.filter((row) => inStage(period, row.stage))) {
    written += (hour.written_gb_h * hour.minutes) / 60;
    minutes += hour.minutes;
  }
  return minutes === 0 ? null : (written / minutes) * 1440;
}

function periodStats(facts, period) {
  const length = spanDays(period);
  const taskRows = facts.taskRows.filter((row) => inStage(period, row.stage));
  const rows = facts.rows.filter((row) => inStage(period, row.stage));
  const tasks = facts.tasks.filter((merge) => inStage(period, merge.stage)).length;
  const prs = facts.rows.filter((row) => inStage(period, row.mergedStage)).length;
  const runs = facts.runs.filter((run) => inStage(period, run.stage));
  const agents = facts.agentRuns.filter((run) => inStage(period, run.stage));
  const workers = agents.filter((run) => run.role === "w" && run.test_s !== null);
  const shell = sum(workers.map((run) => run.test_s + run.build_s + run.lint_s + run.other_s));
  const disk = facts.targets.filter((target) => inStage(period, target.stage)).map((target) => target.gb);
  return {
    length,
    tasks,
    prs,
    tasksADay: length > 0 ? tasks / length : null,
    prsADay: length > 0 ? prs / length : null,
    taskMedian: quantile(taskRows.map((row) => row.total), 0.5),
    task90: quantile(taskRows.map((row) => row.total), 0.9),
    build: quantile(taskRows.map((row) => row.build), 0.5),
    checks: quantile(taskRows.map((row) => row.checks), 0.5),
    queue: quantile(taskRows.map((row) => row.queue), 0.5),
    lines: quantile(taskRows.map((row) => row.lines), 0.5),
    queues: mean(taskRows.map((row) => row.queues)),
    ciPerPr: mean(rows.map((row) => row.runs)),
    failed: runs.length === 0 ? null : runs.filter((run) => run.conclusion === "failure").length / runs.length,
    cancelled: runs.length === 0 ? null : runs.filter((run) => run.conclusion === "cancelled").length / runs.length,
    agentRuns: agents.length,
    agentHours: sum(agents.map((run) => (run.end - run.start) / 3600)),
    tokensPerTask: tasks === 0 ? null : sum(agents.map((run) => run.output_tokens)) / tasks,
    shellHeavy: shell === 0 ? null : sum(workers.map((run) => run.test_s + run.build_s)) / shell,
    target: quantile(disk, 0.5),
    ssd: ssdPerDay(facts, period),
  };
}

function share(value) {
  return `${(100 * value).toFixed(1)}%`;
}

function gigabytes(value) {
  return `${oneDecimal.format(value)} GB`;
}

function cell(value, format, missing) {
  return value === null || value === undefined || Number.isNaN(value) ? `<span class="muted">${missing}</span>` : esc(format(value));
}

function renderStageNote(cutover) {
  const link = `<a href="${GITHUB}/pull/${CUTOVER_PR}">#${CUTOVER_PR}</a>`;
  document.getElementById("stage-note").innerHTML = cutover === null
    ? `Stage 2 starts when ${link} merges. Until then every number here belongs to stage 1.`
    : `Stage 1 ran until ${link} merged on ${esc(longDate.format(cutover * 1000))}. Stage 2 is everything after it.`;
}

function renderStageFigures(facts, stats) {
  const one = stats.get(1);
  const two = facts.cutover === null ? null : stats.get(2);
  const items = [
    ["tasksADay", oneDecimal.format, "Tasks merged a day"],
    ["taskMedian", duration, "Median time from first commit to merged, task PRs"],
    ["ciPerPr", oneDecimal.format, "CI runs per merged PR"],
    ["ssd", gigabytes, "SSD written a day"],
  ];
  document.getElementById("stage-figures").innerHTML = items.map(([key, format, label]) => {
    if (two !== null && two[key] !== null) {
      return figure(esc(format(two[key])), `stage 1: ${one[key] === null ? "none" : esc(format(one[key]))}`, label);
    }
    return figure(one[key] === null ? "none" : esc(format(one[key])), two === null ? "stage 1" : "stage 2: none yet", label);
  }).join("");
}

function renderCompare(facts, stats) {
  const rows = [
    ["Days", "length", oneDecimal.format],
    ["Tasks merged", "tasks", number.format],
    ["Tasks merged a day", "tasksADay", oneDecimal.format],
    ["PRs merged", "prs", number.format],
    ["PRs merged a day", "prsADay", oneDecimal.format],
    ["Task PR, first commit to merged, median", "taskMedian", duration],
    ["Task PR, first commit to merged, 90th percentile", "task90", duration],
    ["Build step, median", "build", duration],
    ["Checks step, median", "checks", duration],
    ["Queue step, median", "queue", duration],
    ["Lines changed per task PR, median", "lines", number.format],
    ["Merge queue tries per task PR", "queues", oneDecimal.format],
    ["CI runs per merged PR", "ciPerPr", oneDecimal.format],
    ["CI runs that failed", "failed", share],
    ["CI runs cancelled", "cancelled", share],
    ["Agent runs", "agentRuns", number.format],
    ["Agent hours", "agentHours", (value) => number.format(Math.round(value))],
    ["Output tokens per merged task", "tokensPerTask", compact.format],
    ["Worker shell time in tests and builds", "shellHeavy", share],
    ["Build folder per task checkout, median", "target", gigabytes],
    ["SSD written a day", "ssd", gigabytes],
  ];
  const head = `<thead><tr><th>Measure</th>${facts.columns.map((period) => `<th class="num">${esc(period.label)}</th>`).join("")}</tr></thead>`;
  const body = rows.map(([label, key, format]) => `<tr><td>${esc(label)}</td>${facts.columns.map((period) => `<td class="num">${cell(stats.get(period.key)[key], format, "none yet")}</td>`).join("")}</tr>`).join("");
  const since = facts.boot ? ` Stage 1 SSD writes start from the laptop's own counter since ${esc(yearDate.format(facts.boot.from * 1000))} and add the 5-minute samples from ${esc(yearDate.format(facts.boot.to * 1000))} on.` : "";
  document.getElementById("stage-compare").innerHTML = `<table>${head}<tbody>${body}</tbody></table><p class="table-note">CI runs count every gate run on the pull request and in the merge queue, cancelled ones included. Worker shell time comes from Claude Code transcripts only.${since}</p>`;
}

function renderPace(facts, stats) {
  const open = facts.entries.filter((entry) => entry.status !== "done").length;
  const week = facts.tasks.filter((merge) => merge.time >= facts.now * 1000 - 7 * DAY).length;
  const paces = [...facts.live.map((period) => [period.label, stats.get(period.key).length, stats.get(period.key).tasks]), ["Last 7 days", 7, week]];
  const rows = paces.map(([label, length, count]) => {
    const rate = length > 0 ? count / length : 0;
    const done = rate > 0 ? yearDate.format(facts.now * 1000 + (open / rate) * DAY) : "never at this pace";
    return `<tr><td>${esc(label)}</td><td class="num">${oneDecimal.format(length)}</td><td class="num">${number.format(count)}</td><td class="num">${oneDecimal.format(rate)}</td><td class="num">${esc(done)}</td></tr>`;
  }).join("");
  document.getElementById("pace").innerHTML = `<table><thead><tr><th>Pace</th><th class="num">Days</th><th class="num">Tasks merged</th><th class="num">A day</th><th class="num">${number.format(open)} open tasks done by</th></tr></thead><tbody>${rows}</tbody></table>`;
}

function renderDailyTable(facts) {
  const taskCounts = countPerDay(facts.days, facts.tasks.map((merge) => merge.time));
  const prCounts = countPerDay(facts.days, facts.rows.map((row) => row.merged * 1000));
  const rows = facts.live.map((period) => {
    const slots = facts.days.map((day, i) => i).filter((i) => facts.days[i] / 1000 >= period.from && facts.days[i] / 1000 + 86400 <= period.to);
    if (slots.length === 0) {
      return `<tr><td>${esc(period.label)}</td><td class="num">0</td><td class="num muted" colspan="5">no whole day yet</td></tr>`;
    }
    const counts = slots.map((i) => taskCounts[i]);
    const busiest = slots.reduce((best, i) => (taskCounts[i] > taskCounts[best] ? i : best), slots[0]);
    return `<tr><td>${esc(period.label)}</td><td class="num">${number.format(slots.length)}</td><td class="num">${oneDecimal.format(sum(counts) / slots.length)}</td><td class="num">${number.format(quantile(counts, 0.5))}</td><td class="num">${number.format(taskCounts[busiest])} on ${esc(shortDate.format(facts.days[busiest]))}</td><td class="num">${number.format(counts.filter((count) => count === 0).length)}</td><td class="num">${oneDecimal.format(sum(slots.map((i) => prCounts[i])) / slots.length)}</td></tr>`;
  }).join("");
  document.getElementById("daily-table").innerHTML = `<table><thead><tr><th>Stage</th><th class="num">Whole days</th><th class="num">Tasks a day</th><th class="num">Median day</th><th class="num">Busiest day</th><th class="num">Days with none</th><th class="num">All PRs a day</th></tr></thead><tbody>${rows}</tbody></table>`;
}

function groupHead(first, periods, names) {
  const top = periods.map((period) => `<th class="num" colspan="${names.length}">${esc(period.label)}</th>`).join("");
  const bottom = periods.map(() => names.map((name) => `<th class="num">${esc(name)}</th>`).join("")).join("");
  return `<thead><tr><th rowspan="2">${esc(first)}</th>${top}</tr><tr>${bottom}</tr></thead>`;
}

function renderSizes(facts) {
  const rows = SIZE_BUCKETS.map((bucket) => {
    const cells = facts.live.map((period) => {
      const inside = facts.taskRows.filter((row) => row.stage === period.key && row.lines !== null && row.lines >= bucket.from && row.lines < bucket.to);
      return `<td class="num">${number.format(inside.length)}</td><td class="num">${cell(quantile(inside.map((row) => row.total), 0.5), duration, "none")}</td><td class="num">${cell(mean(inside.map((row) => row.runs)), oneDecimal.format, "none")}</td>`;
    }).join("");
    return `<tr><td>${esc(bucket.label)}</td>${cells}</tr>`;
  }).join("");
  document.getElementById("sizes").innerHTML = `<table>${groupHead("Lines changed", facts.live, ["PRs", "Median time", "CI runs each"])}<tbody>${rows}</tbody></table>`;
}

function renderCiTable(facts) {
  const rows = CI_EVENTS.map((event) => {
    const cells = facts.live.map((period) => {
      const runs = facts.runs.filter((run) => run.event === event.key && run.stage === period.key);
      const finished = runs.filter((run) => run.conclusion !== "cancelled").map((run) => (run.updated - run.started) / 60);
      const failed = runs.length === 0 ? null : runs.filter((run) => run.conclusion === "failure").length / runs.length;
      return `<td class="num">${number.format(runs.length)}</td><td class="num">${cell(failed, share, "none")}</td><td class="num">${cell(quantile(finished, 0.5), duration, "none")}</td>`;
    }).join("");
    return `<tr><td>${esc(event.label)}</td>${cells}</tr>`;
  }).join("");
  document.getElementById("ci-table").innerHTML = `<table>${groupHead("Event", facts.live, ["Runs", "Failed", "Median time"])}<tbody>${rows}</tbody></table>`;
}

function renderShell(facts) {
  const parts = facts.live.map((period) => {
    const workers = facts.agentRuns.filter((run) => run.role === "w" && run.test_s !== null && run.stage === period.key);
    const seconds = SHELL_KINDS.map((kind) => sum(workers.map((run) => run[kind.key])));
    const total = sum(seconds);
    if (total === 0) {
      return `<h4>${esc(period.label)}</h4><p class="empty">No Claude worker shell time yet.</p>`;
    }
    const bar = SHELL_KINDS.map((kind, i) => (seconds[i] === 0 ? "" : `<span style="flex-grow:${seconds[i]};background:var(${kind.color})" title="${esc(kind.label)}: ${share(seconds[i] / total)}"></span>`)).join("");
    const keys = SHELL_KINDS.map((kind, i) => `<span><i style="background:var(${kind.color})"></i>${esc(kind.label)} <b>${share(seconds[i] / total)}</b> ${oneDecimal.format(seconds[i] / 3600)} h</span>`).join("");
    return `<h4>${esc(period.label)}, ${plural(workers.length, "worker run")}, ${oneDecimal.format(total / 3600)} shell hours</h4><div class="split" aria-hidden="true">${bar}</div><div class="keys">${keys}</div>`;
  });
  document.getElementById("shell").innerHTML = parts.join("");
}

function machineValue(value, unit) {
  return value === null || value === undefined ? `<span class="muted">not sampled</span>` : `${esc(oneDecimal.format(value))}${unit === "%" ? "%" : ""}`;
}

function renderMachine(facts) {
  const hours = facts.hours;
  const full = hours.filter((hour) => hour.minutes >= 45);
  const last = full.at(-1) ?? hours.at(-1) ?? null;
  const columns = [
    { label: "Since start", value: (row) => (facts.boot && row.boot ? row.boot(facts.boot) : null) },
    ...facts.live.map((period) => {
      const inside = hours.filter((hour) => hour.stage === period.key);
      return { label: `${period.label} samples`, value: (row) => (row.key === null || inside.length === 0 ? null : weighted(inside, row.key)) };
    }),
    { label: "Last full hour", value: (row) => (row.key === null || last === null ? null : last[row.key]) },
  ];
  const head = `<thead><tr><th>Measure</th><th>Unit</th>${columns.map((column) => `<th class="num">${esc(column.label)}</th>`).join("")}</tr></thead>`;
  const body = MACHINE_ROWS.map((row) => `<tr><td>${esc(row.label)}</td><td class="muted">${esc(row.unit === "%" ? "" : row.unit)}</td>${columns.map((column) => `<td class="num">${machineValue(column.value(row), row.unit)}</td>`).join("")}</tr>`).join("");
  document.getElementById("machine-table").innerHTML = `<table>${head}<tbody>${body}</tbody></table>`;
  const note = [];
  if (facts.boot) {
    note.push(`Since start covers ${esc(yearDate.format(facts.boot.from * 1000))} to ${esc(yearDate.format(facts.boot.to * 1000))}, read from the laptop's own counters.`);
  }
  note.push(hours.length === 0 ? "No 5-minute sample has been taken yet." : `The 5-minute samples cover ${plural(hours.length, "hour")} up to ${esc(dayTime.format((hours.at(-1).hour + 3600) * 1000))}.`);
  note.push("The SSD numbers count every write on the laptop, not only the agents'.");
  document.getElementById("machine-note").innerHTML = note.join(" ");
  document.getElementById("machine").innerHTML = MACHINE_CHARTS.map((chart) => `<div><h4>${esc(chart.title)}</h4><p>${esc(chart.unit)}</p><div class="chart" data-key="${chart.key}"></div></div>`).join("");
}

function renderMilestonePace(facts) {
  const week = facts.now * 1000 - 7 * DAY;
  const live = facts.live;
  const rows = [...facts.milestones.values()].sort((a, b) => idNumber(a.code) - idNumber(b.code)).map((group) => {
    const merged = facts.tasks.filter((merge) => merge.entry.milestone.split(" ")[0] === group.code);
    const open = group.total - group.done;
    const recent = merged.filter((merge) => merge.time >= week).length;
    const left = open === 0 ? "done" : recent === 0 ? "no merges this week" : `${number.format(Math.ceil(open / (recent / 7)))} days`;
    const rates = live.map((period) => {
      const length = spanDays(period);
      const count = merged.filter((merge) => merge.stage === period.key).length;
      return `<td class="num">${length > 0 ? oneDecimal.format(count / length) : "0.0"}</td>`;
    }).join("");
    return `<tr><td><a href="#milestone-${esc(group.code)}">${esc(group.name)}</a></td><td class="num">${number.format(open)}</td>${rates}<td class="num">${number.format(recent)}</td><td class="num">${esc(left)}</td></tr>`;
  }).join("");
  const rateHeads = live.map((period) => `<th class="num">${esc(period.label)} a day</th>`).join("");
  document.getElementById("milestone-pace").innerHTML = `<table><thead><tr><th>Milestone</th><th class="num">Open</th>${rateHeads}<th class="num">Last 7 days</th><th class="num">Days left</th></tr></thead><tbody>${rows}</tbody></table>`;
}

// Without the cursor on the chart, the legend shows the last point instead of dashes.
function legendAtEnd(chart) {
  if (chart.cursor.idx == null) {
    chart.setLegend({ idx: chart.data[0].length - 1 });
  }
}

function paintStageBand(chart, cutover) {
  const { left, top, width, height } = chart.bbox;
  const from = Math.max(left, chart.valToPos(cutover, "x", true));
  if (from < left + width) {
    chart.ctx.save();
    chart.ctx.fillStyle = `${cssColor("--c4")}14`;
    chart.ctx.fillRect(from, top, left + width - from, height);
    chart.ctx.restore();
  }
}

function paintStageLine(chart, cutover) {
  const { left, top, width, height } = chart.bbox;
  const x = Math.round(chart.valToPos(cutover, "x", true));
  if (x < left || x > left + width) {
    return;
  }
  const ratio = window.devicePixelRatio || 1;
  const ctx = chart.ctx;
  const flip = x > left + width * 0.8;
  ctx.save();
  ctx.strokeStyle = cssColor("--ink");
  ctx.lineWidth = ratio;
  ctx.setLineDash([4 * ratio, 3 * ratio]);
  ctx.beginPath();
  ctx.moveTo(x, top);
  ctx.lineTo(x, top + height);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = cssColor("--muted");
  ctx.font = `${11 * ratio}px 'IBM Plex Mono', ui-monospace, monospace`;
  ctx.textAlign = flip ? "right" : "left";
  ctx.textBaseline = "top";
  ctx.fillText("stage 2", x + (flip ? -4 : 4) * ratio, top + 4 * ratio);
  ctx.restore();
}

function axis() {
  return {
    stroke: cssColor("--muted"),
    font: "11px 'IBM Plex Mono', ui-monospace, monospace",
    grid: { stroke: cssColor("--faint"), width: 1 },
    ticks: { stroke: cssColor("--rule"), width: 1, size: 4 },
  };
}

// Midnight ticks show the date, the ones between show the hour.
function timeLabels(chart, splits) {
  return splits.map((split) => {
    const date = new Date(split * 1000);
    return date.getHours() === 0 ? shortDate.format(date) : hourOnly.format(date);
  });
}

function chartOptions(container, height, colors, first, incrs, cutover) {
  const options = {
    width: Math.max(320, container.clientWidth),
    height,
    cursor: { y: false, sync: { key: "history" }, drag: { x: true, y: false } },
    legend: { markers: { width: 0, fill: (chart, i) => colors[i] } },
    scales: { x: { time: true }, y: { range: (chart, min, max) => [0, uPlot.rangeNum(0, Math.max(max ?? 0, 1), 0.1, true)[1]] } },
    axes: [{ ...axis(), space: 60, incrs, values: timeLabels }, { ...axis(), size: 48 }],
    series: [first],
    hooks: { ready: [legendAtEnd], setCursor: [legendAtEnd] },
  };
  if (cutover !== null) {
    options.hooks.drawClear = [(chart) => paintStageBand(chart, cutover)];
    options.hooks.draw = [(chart) => paintStageLine(chart, cutover)];
  }
  return options;
}

function dayLabel(chart, value) {
  return value == null ? null : shortDate.format(value * 1000);
}

function drawTotals(container, days, view, cutover) {
  const colors = [null, ...view.map((series) => cssColor(series.color))];
  const data = [days.map((day) => day / 1000)];
  for (const series of view) {
    const totals = [];
    let total = 0;
    for (const count of countPerDay(days, series.times)) {
      total += count;
      totals.push(total);
    }
    data.push(totals);
  }
  const options = chartOptions(container, 260, colors, { label: "Day", value: dayLabel }, DAY_STEPS, cutover);
  view.forEach((series, i) => {
    options.series.push({
      label: series.label,
      stroke: colors[i + 1],
      width: 2,
      fill: view.length === 1 ? `${colors[i + 1]}1f` : undefined,
      points: { show: false },
      value: (chart, value) => (value == null ? null : number.format(value)),
    });
  });
  return new uPlot(options, data, container);
}

// A series' bar reaches its own count plus every shown series after it. Taller bars draw first, so later ones cover their lower part.
function stackBars(counts, shown) {
  const stacked = [];
  let below = counts[0].map(() => 0);
  for (let k = counts.length - 1; k >= 0; k--) {
    if (shown[k]) {
      below = below.map((total, i) => total + counts[k][i]);
    }
    stacked[k] = shown[k] ? below : counts[k];
  }
  return stacked;
}

function dailyData(x, counts, shown) {
  const totals = x.map((value, i) => sum(counts.map((series, k) => (shown[k] ? series[i] : 0))));
  const average = totals.map((value, i) => mean(totals.slice(Math.max(0, i - 6), i + 1)));
  return [x, ...stackBars(counts, shown), average];
}

function drawDaily(container, days, view, cutover, format) {
  const ink = cssColor("--ink");
  const colors = [null, ...view.map((series) => cssColor(series.color)), ink];
  const counts = view.map((series) => countPerDay(days, series.times, series.weights));
  const x = days.map((day) => day / 1000);
  const options = chartOptions(container, 220, colors, { label: "Day", value: dayLabel }, DAY_STEPS, cutover);
  options.scales.x.range = (chart, min, max) => [min - 43200, max + 43200];
  options.cursor.points = { show: false };
  const bars = uPlot.paths.bars({ size: [0.7, 48] });
  view.forEach((series, i) => {
    options.series.push({
      label: series.label,
      fill: colors[i + 1],
      stroke: cssColor("--paper"),
      width: 1,
      paths: bars,
      points: { show: false },
      value: (chart, value, seriesIdx, idx) => (idx == null ? null : format(counts[seriesIdx - 1][idx])),
    });
  });
  options.series.push({ label: "7-day average", stroke: ink, width: 2, points: { show: false }, value: (chart, value) => (value == null ? null : oneDecimal.format(value)) });
  options.hooks.setSeries = [(chart) => chart.setData(dailyData(x, counts, chart.series.slice(1, -1).map((series) => series.show)))];
  return new uPlot(options, dailyData(x, counts, view.map(() => true)), container);
}

function drawStages(container, rows, cutover) {
  const paper = cssColor("--paper");
  const colors = [null];
  const raw = [null];
  const data = [rows.map((row) => row.x)];
  const label = (chart, value, seriesIdx, idx) => (idx == null ? null : `#${rows[idx].number}${rows[idx].task ? ` ${rows[idx].task}` : ""}, ${dayTime.format(rows[idx].merged * 1000)}`);
  const value = (chart, plotted, seriesIdx, idx) => (idx == null || raw[seriesIdx][idx] === null ? null : duration(raw[seriesIdx][idx]));
  const options = chartOptions(container, 320, colors, { label: "Pull request", value: label }, HOUR_STEPS, cutover);
  for (const step of STEPS) {
    const color = cssColor(step.color);
    const values = rows.map((row) => row[step.key]);
    const medians = [];
    for (let i = 0; i < values.length; i++) {
      medians.push(quantile(values.slice(Math.max(0, i - MEDIAN_WINDOW + 1), i + 1), 0.5));
    }
    colors.push(color, color);
    raw.push(values, medians);
    // The scale is logarithmic, so anything under a minute sits on the one-minute line.
    data.push(values.map((minutes) => (minutes === null ? null : Math.max(1, minutes))));
    data.push(medians.map((minutes) => (minutes === null ? null : Math.max(1, minutes))));
    options.series.push(
      { label: step.label, stroke: color, width: 0, paths: () => null, points: { show: true, size: 8, width: 1, stroke: paper, fill: `${color}80` }, value },
      { label: `${step.label} median`, stroke: color, width: 2, points: { show: false }, value },
    );
  }
  options.cursor.points = { size: 12, width: 2, stroke: (chart, i) => colors[i], fill: paper };
  options.scales.y = { distr: 3, log: 10, range: (chart, min, max) => [1, STAGE_TICKS.find((tick) => tick >= max) ?? max] };
  options.axes[1] = {
    ...axis(),
    size: 64,
    splits: (chart, axisIdx, min, max) => STAGE_TICKS.filter((tick) => tick >= min && tick <= max),
    // uPlot labels only powers of ten on a log scale unless the filter keeps every tick.
    filter: (chart, splits) => splits,
    values: (chart, splits) => splits.map(duration),
  };
  return new uPlot(options, data, container);
}

function renderStageTable(rows, facts) {
  const cells = [...STEPS, TOTAL].map((step) => {
    const values = facts.columns.map((period) => {
      const inside = rows.filter((row) => inStage(period, row.stage)).map((row) => row[step.key]);
      return `<td class="num">${cell(quantile(inside, 0.5), duration, "none yet")}</td><td class="num">${cell(quantile(inside, 0.9), duration, "none yet")}</td>`;
    }).join("");
    return `<tr><td>${esc(step.label)}</td><td class="muted">${esc(step.about)}</td>${values}</tr>`;
  });
  const top = facts.columns.map((period) => `<th class="num" colspan="2">${esc(period.label)}</th>`).join("");
  const bottom = facts.columns.map(() => `<th class="num">Median</th><th class="num">90th</th>`).join("");
  document.getElementById("stage-table").innerHTML = `<table><thead><tr><th rowspan="2">Step</th><th rowspan="2">From and to</th>${top}</tr><tr>${bottom}</tr></thead><tbody>${cells.slice(0, -1).join("")}</tbody><tfoot>${cells.at(-1)}</tfoot></table>`;
  const counted = `${number.format(rows.length)} merged since ${shortDate.format(rows[0].merged * 1000)}`;
  document.getElementById("stage-count").textContent = facts.cutover === null
    ? `${counted}.`
    : `${counted}: ${number.format(rows.filter((row) => row.stage === 1).length)} in stage 1, ${number.format(rows.filter((row) => row.stage === 2).length)} started in stage 2 and ${number.format(rows.filter((row) => row.stage === 0).length)} started before the switch and merged after it.`;
}

function hourValues(facts, period, mode) {
  const slots = mode === "hour" ? 24 : 7;
  const counts = new Array(slots).fill(0);
  for (const row of facts.rows.filter((item) => item.mergedStage === period.key)) {
    const date = new Date(row.merged * 1000);
    counts[mode === "hour" ? date.getHours() : (date.getDay() + 6) % 7] += 1;
  }
  if (mode === "hour") {
    return counts.map((count) => count / Math.max(1, spanDays(period)));
  }
  const weekdays = new Array(7).fill(0);
  for (const day of facts.days.filter((start) => start / 1000 + 86400 > period.from && start / 1000 < period.to)) {
    weekdays[(new Date(day).getDay() + 6) % 7] += 1;
  }
  return counts.map((count, i) => (weekdays[i] === 0 ? 0 : count / weekdays[i]));
}

function slotName(mode, slot) {
  return mode === "hour" ? hourOnly.format(new Date(2026, 0, 1, slot)) : WEEKDAYS[slot];
}

function drawHours(container, facts, mode) {
  const slots = mode === "hour" ? 24 : 7;
  const values = facts.live.map((period) => hourValues(facts, period, mode));
  const colors = [null, ...facts.live.map((period) => cssColor(STAGE_COLORS[period.key]))];
  const options = {
    width: Math.max(320, container.clientWidth),
    height: 220,
    cursor: { y: false, points: { show: false }, drag: { x: false, y: false } },
    legend: { markers: { width: 0, fill: (chart, i) => colors[i] } },
    scales: { x: { time: false, range: () => [-0.5, slots - 0.5] }, y: { range: (chart, min, max) => [0, uPlot.rangeNum(0, Math.max(max ?? 0, 1), 0.1, true)[1]] } },
    axes: [{ ...axis(), space: 34, incrs: [1, 2, 3, 6], values: (chart, splits) => splits.map((split) => slotName(mode, split)) }, { ...axis(), size: 48 }],
    series: [{ label: mode === "hour" ? "Hour" : "Weekday", value: (chart, value) => (value == null ? null : slotName(mode, value)) }],
  };
  facts.live.forEach((period, i) => {
    const align = facts.live.length === 1 ? 0 : i === 0 ? -1 : 1;
    options.series.push({
      label: `${period.label}, merges a day`,
      fill: colors[i + 1],
      stroke: colors[i + 1],
      width: 0,
      paths: uPlot.paths.bars({ size: facts.live.length === 1 ? [0.7, 48] : [0.4, 40], align }),
      points: { show: false },
      value: (chart, value) => (value == null ? null : value.toFixed(2)),
    });
  });
  const table = [...Array(slots).keys()].map((slot) => `<tr><td>${esc(slotName(mode, slot))}</td>${values.map((series) => `<td class="num">${series[slot].toFixed(2)}</td>`).join("")}</tr>`).join("");
  document.getElementById("hours-table").innerHTML = `<table><thead><tr><th>${mode === "hour" ? "Hour" : "Weekday"}</th>${facts.live.map((period) => `<th class="num">${esc(period.label)}, merges a day</th>`).join("")}</tr></thead><tbody>${table}</tbody></table>`;
  return new uPlot(options, [[...Array(slots).keys()], ...values], container);
}

function ciView(facts, mode) {
  const groups = mode === "result" ? CI_RESULTS : CI_EVENTS;
  const field = mode === "result" ? "conclusion" : "event";
  return groups.map((group) => ({ label: group.label, color: group.color, times: facts.runs.filter((run) => run[field] === group.key).map((run) => run.created * 1000) }));
}

function agentView(facts, mode) {
  const counts = new Map();
  for (const run of facts.agentRuns) {
    counts.set(run.model, (counts.get(run.model) ?? 0) + 1);
  }
  const top = [...counts.keys()].sort((a, b) => counts.get(b) - counts.get(a)).slice(0, 6).sort();
  const groups = top.map((model, i) => ({ label: model, color: `--c${i + 1}`, runs: facts.agentRuns.filter((run) => run.model === model) }));
  const rest = facts.agentRuns.filter((run) => !top.includes(run.model));
  if (rest.length > 0) {
    groups.push({ label: "Other", color: "--other", runs: rest });
  }
  return groups.map((group) => ({
    label: group.label,
    color: group.color,
    times: group.runs.map((run) => run.start * 1000),
    weights: mode === "hours" ? group.runs.map((run) => (run.end - run.start) / 3600) : null,
  }));
}

function drawMachine(charts, facts) {
  const hours = facts.hours;
  const x = hours.map((hour) => hour.hour);
  for (const container of document.querySelectorAll("#machine .chart")) {
    const spec = MACHINE_CHARTS.find((chart) => chart.key === container.dataset.key);
    if (hours.length === 0) {
      container.innerHTML = `<p class="empty">Not sampled yet.</p>`;
      continue;
    }
    const reference = MACHINE_ROWS.find((row) => row.key === spec.key).boot;
    const color = cssColor("--c1");
    const muted = cssColor("--muted");
    const colors = [null, color, muted];
    const options = chartOptions(container, 140, colors, { label: "Hour", value: (chart, value) => (value == null ? null : dayTime.format(value * 1000)) }, HOUR_STEPS, facts.cutover);
    options.cursor.sync.key = "machine";
    options.axes[1].size = 40;
    options.series.push({ label: spec.title, stroke: color, width: 2, points: { show: hours.length < 48, size: 5 }, value: (chart, value) => (value == null ? null : oneDecimal.format(value)) });
    const data = [x, hours.map((hour) => hour[spec.key])];
    if (facts.boot && reference) {
      const level = reference(facts.boot);
      options.series.push({ label: "Since start", stroke: muted, width: 1, dash: [4, 3], points: { show: false }, value: () => oneDecimal.format(level) });
      data.push(hours.map(() => level));
    }
    charts.set(container, new uPlot(options, data, container));
  }
}

function drawHistory(charts, facts) {
  for (const chart of charts.values()) {
    chart.destroy();
  }
  const ids = ["burnup", "daily", "hours", "stages", "ci", "agents"];
  const [burnup, daily, hours, stages, ci, agents] = ids.map((id) => document.getElementById(id));
  charts.set(burnup, drawTotals(burnup, facts.days, facts.views[pressed("burnup")], facts.cutover));
  charts.set(daily, drawDaily(daily, facts.days, facts.views[pressed("daily")], facts.cutover, number.format));
  charts.set(hours, drawHours(hours, facts, pressed("hours")));
  const rows = pressed("stages") === "tasks" ? facts.taskRows : facts.rows;
  charts.set(stages, drawStages(stages, rows, facts.cutover));
  renderStageTable(rows, facts);
  charts.set(ci, drawDaily(ci, facts.days, ciView(facts, pressed("ci")), facts.cutover, number.format));
  const agentMode = pressed("agents");
  charts.set(agents, drawDaily(agents, facts.days, agentView(facts, agentMode), facts.cutover, agentMode === "hours" ? oneDecimal.format : number.format));
  drawMachine(charts, facts);
}

function stageLanes(facts, select) {
  return facts.live.map((period) => {
    const all = select(period.key);
    return { label: period.label, color: STAGE_COLORS[period.key], values: all.filter((value) => value > 0), zeros: all.filter((value) => value <= 0).length };
  });
}

function violinMetrics(facts, mode) {
  const minutes = (value) => Math.max(1, value);
  const ciTicks = [1, 2, 5, 10, 20, 30, 60, 120, 240];
  const counts = [1, 3, 10, 30, 100, 300, 1000, 3000, 10000, 30000];
  if (mode === "time") {
    return [...STEPS, TOTAL].map((step) => ({
      title: step.label === "Total" ? "Total, first commit to merged" : `${step.label} step`,
      ticks: STAGE_TICKS,
      format: duration,
      lanes: stageLanes(facts, (stage) => facts.taskRows.filter((row) => row.stage === stage && row[step.key] !== null).map((row) => minutes(row[step.key]))),
    }));
  }
  if (mode === "size") {
    return [["lines", "Lines changed"], ["files", "Files changed"], ["commits", "Commits"]].map(([key, title]) => ({
      title,
      ticks: counts,
      format: number.format,
      lanes: stageLanes(facts, (stage) => facts.taskRows.filter((row) => row.stage === stage && row[key] !== null).map((row) => row[key])),
    }));
  }
  if (mode === "ci") {
    return CI_EVENTS.map((event) => ({
      title: `${event.label} runs`,
      ticks: ciTicks,
      format: duration,
      lanes: stageLanes(facts, (stage) => facts.runs.filter((run) => run.event === event.key && run.stage === stage && run.conclusion !== "cancelled").map((run) => minutes((run.updated - run.started) / 60))),
    }));
  }
  if (mode === "agents") {
    return [
      { title: "Worker run time", ticks: STAGE_TICKS, format: duration, lanes: stageLanes(facts, (stage) => facts.agentRuns.filter((run) => run.role === "w" && run.stage === stage).map((run) => minutes((run.end - run.start) / 60))) },
      { title: "Reviewer run time", ticks: STAGE_TICKS, format: duration, lanes: stageLanes(facts, (stage) => facts.agentRuns.filter((run) => run.role === "r" && run.stage === stage).map((run) => minutes((run.end - run.start) / 60))) },
      { title: "Output tokens per worker run", ticks: [1000, 3000, 10000, 30000, 100000, 300000, 1000000], format: compact.format, lanes: stageLanes(facts, (stage) => facts.agentRuns.filter((run) => run.role === "w" && run.stage === stage && run.output_tokens !== null).map((run) => run.output_tokens)) },
    ];
  }
  return [{ title: "Build folder per task checkout", ticks: [0.1, 0.3, 1, 3, 10, 30], format: gigabytes, lanes: stageLanes(facts, (stage) => facts.targets.filter((target) => target.stage === stage).map((target) => target.gb)) }];
}

function violinX(scale, log) {
  return scale.left + ((log - scale.lo) / (scale.hi - scale.lo)) * (scale.right - scale.left);
}

function violinTip(lane, metric) {
  const lines = [`${lane.label}, ${metric.title}`, `${plural(lane.values.length, "value")}${lane.zeros > 0 ? `, ${number.format(lane.zeros)} zeros left out` : ""}`];
  for (const [name, at] of [["25th", 0.25], ["median", 0.5], ["75th", 0.75], ["90th", 0.9]]) {
    lines.push(`${name.padEnd(7)} ${metric.format(quantile(lane.values, at))}`);
  }
  return lines.join("\n");
}

function violinLane(lane, index, metric, scale, width) {
  const top = index * VIOLIN.lane;
  const mid = top + VIOLIN.lane / 2;
  const name = `<text class="lane-name" x="0" y="${mid}" dominant-baseline="middle">${esc(lane.label)}</text>`;
  if (lane.values.length === 0) {
    return `${name}<text x="${scale.left}" y="${mid}" dominant-baseline="middle">none yet</text>`;
  }
  const logs = lane.values.map(Math.log10);
  const average = sum(logs) / logs.length;
  const spread = Math.sqrt(sum(logs.map((value) => (value - average) ** 2)) / logs.length);
  const bandwidth = Math.max(0.04, 1.06 * spread * logs.length ** -0.2);
  const density = [];
  for (let i = 0; i < VIOLIN.points; i++) {
    const at = scale.lo + (i / (VIOLIN.points - 1)) * (scale.hi - scale.lo);
    let total = 0;
    for (const value of logs) {
      const z = (at - value) / bandwidth;
      total += Math.exp(-0.5 * z * z);
    }
    density.push([violinX(scale, at), total]);
  }
  const peak = Math.max(...density.map(([, total]) => total));
  const half = VIOLIN.lane / 2 - 3;
  const upper = density.map(([x, total]) => `${x.toFixed(1)},${(mid - (half * total) / peak).toFixed(1)}`);
  const lower = density.map(([x, total]) => `${x.toFixed(1)},${(mid + (half * total) / peak).toFixed(1)}`).reverse();
  const [x25, x50, x75] = [0.25, 0.5, 0.75].map((at) => violinX(scale, Math.log10(quantile(lane.values, at))));
  return `${name}<path d="M${upper.join("L")}L${lower.join("L")}Z" style="fill:var(${lane.color});stroke:var(${lane.color})" fill-opacity="0.4" stroke-width="1"/>`
    + `<rect class="box" x="${x25.toFixed(1)}" y="${mid - 3}" width="${Math.max(2, x75 - x25).toFixed(1)}" height="6"/>`
    + `<line class="median" x1="${x50.toFixed(1)}" x2="${x50.toFixed(1)}" y1="${mid - 3}" y2="${mid + 3}"/>`
    + `<rect class="hit" x="0" y="${top}" width="${width}" height="${VIOLIN.lane}" data-tip="${esc(violinTip(lane, metric))}"/>`;
}

function violinBlock(metric, width) {
  const all = metric.lanes.flatMap((lane) => lane.values);
  if (all.length === 0) {
    return `<div class="violin"><p>${esc(metric.title)}</p><p class="empty">No values yet.</p></div>`;
  }
  let lo = Math.log10(Math.min(...all));
  let hi = Math.log10(Math.max(...all));
  const pad = Math.max(0.05, (hi - lo) * 0.04);
  lo -= pad;
  hi += pad;
  const scale = { lo, hi, left: VIOLIN.label, right: width - 8 };
  const height = metric.lanes.length * VIOLIN.lane + VIOLIN.axis;
  const grid = [];
  let previous = -Infinity;
  for (const tick of metric.ticks.filter((value) => Math.log10(value) >= lo && Math.log10(value) <= hi)) {
    const x = violinX(scale, Math.log10(tick));
    if (x - previous >= VIOLIN.gap) {
      grid.push(`<line class="grid" x1="${x.toFixed(1)}" x2="${x.toFixed(1)}" y1="0" y2="${height - VIOLIN.axis}"/><text x="${x.toFixed(1)}" y="${height - 4}" text-anchor="middle">${esc(metric.format(tick))}</text>`);
      previous = x;
    }
  }
  const lanes = metric.lanes.map((lane, i) => violinLane(lane, i, metric, scale, width)).join("");
  return `<div class="violin"><p>${esc(metric.title)}</p><svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-label="${esc(metric.title)}">${grid.join("")}${lanes}</svg></div>`;
}

function violinTable(metrics) {
  const shares = [0.1, 0.25, 0.5, 0.75, 0.9];
  const rows = metrics.flatMap((metric) => metric.lanes.map((lane) => {
    const cells = shares.map((at) => `<td class="num">${lane.values.length === 0 ? "" : esc(metric.format(quantile(lane.values, at)))}</td>`).join("");
    return `<tr><td>${esc(metric.title)}</td><td>${esc(lane.label)}</td><td class="num">${number.format(lane.values.length)}</td>${cells}</tr>`;
  })).join("");
  return `<table><thead><tr><th>Values</th><th>Stage</th><th class="num">Count</th><th class="num">10th</th><th class="num">25th</th><th class="num">Median</th><th class="num">75th</th><th class="num">90th</th></tr></thead><tbody>${rows}</tbody></table>`;
}

function drawViolins(facts) {
  const container = document.getElementById("violins");
  const mode = pressed("violins");
  const metrics = violinMetrics(facts, mode);
  const width = Math.floor(container.clientWidth);
  document.getElementById("violin-about").textContent = VIOLIN_ABOUT[mode];
  container.innerHTML = width === 0 ? "" : metrics.map((metric) => violinBlock(metric, width)).join("");
  document.getElementById("violin-table").innerHTML = violinTable(metrics);
  return width;
}

function showTip(tip, event) {
  const hit = event.target.closest(".hit");
  if (!hit) {
    tip.hidden = true;
    return;
  }
  tip.textContent = hit.dataset.tip;
  tip.hidden = false;
  const x = Math.max(8, Math.min(event.clientX + 14, window.innerWidth - tip.offsetWidth - 8));
  const y = event.clientY + 14 + tip.offsetHeight > window.innerHeight ? event.clientY - tip.offsetHeight - 10 : event.clientY + 14;
  tip.style.left = `${x}px`;
  tip.style.top = `${y}px`;
}

export function renderHistory(data) {
  const facts = historyFacts(data);
  renderStageNote(facts.cutover);
  renderMilestonePace(facts);
  if (facts.rows.length === 0) {
    for (const id of ["burnup", "daily", "hours", "stages", "violins", "ci", "agents"]) {
      document.getElementById(id).innerHTML = `<p class="empty">No pull request has merged yet.</p>`;
    }
    return;
  }
  const stats = new Map(facts.periods.map((period) => [period.key, periodStats(facts, period)]));
  renderStageFigures(facts, stats);
  renderCompare(facts, stats);
  renderPace(facts, stats);
  renderDailyTable(facts);
  renderSizes(facts);
  renderCiTable(facts);
  renderShell(facts);
  renderMachine(facts);

  const charts = new Map();
  drawHistory(charts, facts);
  const violins = document.getElementById("violins");
  let violinWidth = drawViolins(facts);
  const tip = document.createElement("div");
  tip.className = "tip";
  tip.hidden = true;
  document.body.append(tip);
  violins.addEventListener("pointermove", (event) => showTip(tip, event));
  violins.addEventListener("pointerdown", (event) => showTip(tip, event));
  violins.addEventListener("pointerleave", () => {
    tip.hidden = true;
  });

  for (const group of document.querySelectorAll("#history .modes")) {
    group.addEventListener("click", (event) => {
      const button = event.target.closest("button");
      if (button) {
        for (const other of group.querySelectorAll("button")) {
          other.setAttribute("aria-pressed", String(other === button));
        }
        drawHistory(charts, facts);
        violinWidth = drawViolins(facts);
      }
    });
  }
  // A chart reads its colors when it is drawn, so a theme change draws it again.
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    drawHistory(charts, facts);
    violinWidth = drawViolins(facts);
  });
  // A hidden tab has no width, so a chart takes its width when its tab shows, and again on every resize.
  const resizer = new ResizeObserver((records) => {
    for (const record of records) {
      const width = Math.floor(record.contentRect.width);
      if (record.target === violins) {
        if (width > 0 && width !== violinWidth) {
          violinWidth = drawViolins(facts);
        }
      } else if (width > 0 && charts.has(record.target)) {
        const chart = charts.get(record.target);
        chart.setSize({ width, height: chart.height });
      }
    }
  });
  for (const container of [...charts.keys(), violins]) {
    resizer.observe(container);
  }
}
