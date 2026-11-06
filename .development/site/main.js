import { REPO, GITHUB, DAY, number, shortDate, longDate, esc, percent, idNumber, figure, plural, fetchText, parseCsv } from "./shared.js";
import { renderHistory } from "./history.js";

const STATES = [
  { key: "done", label: "Done" },
  { key: "review", label: "In review" },
  { key: "ready", label: "Ready" },
  { key: "waiting", label: "Waiting" },
  { key: "unplanned", label: "No task file" },
];

// First match wins, so the narrower folders come before litellm/proxy/.
const AREAS = [
  { name: "Providers", prefixes: ["litellm/llms/"] },
  { name: "Guardrails", prefixes: ["litellm/proxy/guardrails/", "litellm/guardrails"] },
  { name: "Proxy server, keys and spend", prefixes: ["litellm/proxy/"] },
  { name: "Router", prefixes: ["litellm/router"] },
  { name: "Observability", prefixes: ["litellm/integrations/"] },
  { name: "Core", prefixes: ["litellm/litellm_core_utils/", "litellm/main.py", "litellm/utils.py", "litellm/cost_calculator", "litellm/exceptions", "litellm/types/"] },
  { name: "Everything else", prefixes: [""] },
];

const PATHS = [
  {
    name: "Router and Anthropic",
    about: "A chat request goes through the router, picks a deployment, retries within its deadline and gets its answer from Anthropic.",
    targets: ["B016", "B017", "B027"],
  },
  {
    name: "Keys, spend and logging",
    about: "The same request behind virtual keys, with its cost written to spend tracking and the call logged.",
    targets: ["B016", "B017", "B027", "B014", "B052", "B068", "B070", "B012"],
  },
];

function taskLink(entry) {
  const href = entry.pr ? `${GITHUB}/pull/${entry.pr}` : `${GITHUB}/blob/main/.development/backlog/${entry.id}.yaml`;
  return `<a class="id" href="${esc(href)}">${esc(entry.id)}</a>`;
}

function progressBar(part, whole) {
  const share = whole === 0 ? 0 : (100 * part) / whole;
  return `<div class="bar" role="img" aria-label="${percent(part, whole)} done"><div class="track"><div class="fill" style="width:${share.toFixed(2)}%"></div></div><span class="pct">${percent(part, whole)}</span></div>`;
}

// Globs follow globset with literal_separator: * and ? stay inside one path segment, ** crosses them.
function globToRegExp(glob) {
  let source = "";
  let braces = 0;
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*" && glob[i + 1] === "*" && glob[i + 2] === "/") {
      source += "(?:.*/)?";
      i += 2;
    } else if (c === "*" && glob[i + 1] === "*") {
      source += ".*";
      i += 1;
    } else if (c === "*") {
      source += "[^/]*";
    } else if (c === "?") {
      source += "[^/]";
    } else if (c === "[" && glob.indexOf("]", i + 2) !== -1) {
      const end = glob.indexOf("]", i + 2);
      const body = glob.slice(i + 1, end).replace(/\\/g, "\\\\");
      source += body.startsWith("!") ? `[^${body.slice(1)}]` : `[${body}]`;
      i = end;
    } else if (c === "{") {
      source += "(?:";
      braces += 1;
    } else if (c === "}" && braces > 0) {
      source += ")";
      braces -= 1;
    } else if (c === "," && braces > 0) {
      source += "|";
    } else {
      source += c.replace(/[.+^$()|\\[\]{}]/g, "\\$&");
    }
  }
  return new RegExp(`^${source}$`);
}

function depthOf(id, byId, depths, next) {
  if (depths.has(id)) {
    return depths.get(id);
  }
  depths.set(id, 0);
  const entry = byId.get(id);
  let best = 0;
  for (const dep of entry.depends_on ?? []) {
    if (!byId.has(dep)) {
      continue;
    }
    const depth = depthOf(dep, byId, depths, next);
    if (depth > best) {
      best = depth;
      next.set(id, dep);
    }
  }
  const depth = best + (entry.status === "done" ? 0 : 1);
  depths.set(id, depth);
  return depth;
}

// Every task the path's targets need, through depends_on, and the longest run of open tasks among them.
function analyzePath(path, byId, depths, next) {
  const members = new Set();
  const stack = [...path.targets];
  while (stack.length > 0) {
    const id = stack.pop();
    if (members.has(id) || !byId.has(id)) {
      continue;
    }
    members.add(id);
    stack.push(...(byId.get(id).depends_on ?? []));
  }
  const entries = [...members].map((id) => byId.get(id));
  const open = entries.filter((entry) => entry.status !== "done").sort((a, b) => idNumber(a.id) - idNumber(b.id));

  let head = path.targets[0];
  for (const id of path.targets) {
    if (depthOf(id, byId, depths, next) > depthOf(head, byId, depths, next)) {
      head = id;
    }
  }
  const chain = [];
  for (let id = head; id !== undefined; id = next.get(id)) {
    if (byId.get(id).status !== "done") {
      chain.push(byId.get(id));
    }
  }
  chain.reverse();
  return { path, total: entries.length, open, chain, withoutFile: open.filter((entry) => !entry.file).length };
}

// The first five rows, and the rest behind a toggle.
function shortTable(head, rows, rest) {
  const shown = `<div class="table-scroll"><table>${head}<tbody>${rows.slice(0, 5).join("")}</tbody></table></div>`;
  if (rows.length <= 5) {
    return shown;
  }
  return `${shown}<details><summary>Show ${number.format(rows.length - 5)} ${rest}</summary><div class="table-scroll"><table>${head}<tbody>${rows.slice(5).join("")}</tbody></table></div></details>`;
}

function taskRow(entry) {
  return `<tr><td>${taskLink(entry)}</td><td>${esc(entry.title)}</td></tr>`;
}

function renderPath(facts, byId, reviewing) {
  const { path, total, open, chain, withoutFile } = facts;
  const done = total - open.length;
  const ready = open.filter((entry) => taskState(entry, byId, reviewing) === "ready");
  const inReview = open.filter((entry) => reviewing.has(entry.id)).length;
  const targets = path.targets.map((id) => taskLink(byId.get(id))).join(", ");
  const readyRows = ready.map((entry) => `<tr><td>${taskLink(entry)}</td><td>${esc(entry.title)}</td><td class="muted">${esc(entry.milestone)}</td></tr>`);
  const chainRows = chain.map((entry, i) => `<tr><td class="num muted">${i + 1}</td><td>${taskLink(entry)}</td><td>${esc(entry.title)}</td><td class="muted">${esc(entry.milestone)}</td></tr>`).join("");
  const readyTable = ready.length === 0
    ? `<p class="empty">Nothing on this path can start yet.</p>`
    : shortTable(`<thead><tr><th>Task</th><th>Title</th><th>Milestone</th></tr></thead>`, readyRows, "more");
  const chainTable = chain.length === 0
    ? ""
    : `<details><summary>Longest open chain, ${number.format(chain.length)} tasks, first step first</summary><div class="table-scroll"><table><thead><tr><th class="num">Step</th><th>Task</th><th>Title</th><th>Milestone</th></tr></thead><tbody>${chainRows}</tbody></table></div></details>`;

  return `<div class="path">
    <h4>${esc(path.name)}</h4>
    <p>${esc(path.about)} Done when ${targets} merge.</p>
    ${progressBar(done, total)}
    <p>${number.format(done)} of ${number.format(total)} tasks merged, dependencies included. The longest open chain is ${number.format(chain.length)} tasks, ${number.format(withoutFile)} still need a task file and ${number.format(inReview)} ${inReview === 1 ? "is" : "are"} in review. These ${number.format(ready.length)} are ready to start:</p>
    ${readyTable}
    ${chainTable}
  </div>`;
}

function taskState(entry, byId, reviewing) {
  if (entry.status === "done") {
    return "done";
  }
  if (reviewing.has(entry.id)) {
    return "review";
  }
  if (!entry.file) {
    return "unplanned";
  }
  return (entry.depends_on ?? []).some((dep) => byId.has(dep) && byId.get(dep).status !== "done") ? "waiting" : "ready";
}

function renderMilestone(group, milestones, byId, children, merges, reviewing) {
  const byState = new Map(STATES.map((state) => [state.key, []]));
  for (const entry of [...group.entries].sort((a, b) => idNumber(a.id) - idNumber(b.id))) {
    byState.get(taskState(entry, byId, reviewing)).push(entry);
  }
  const bar = STATES.filter((state) => byState.get(state.key).length > 0)
    .map((state) => `<span class="state-${state.key}" style="flex-grow:${byState.get(state.key).length}" title="${state.label}: ${number.format(byState.get(state.key).length)}"></span>`)
    .join("");
  const keys = STATES.map((state) => `<span><i class="state-${state.key}"></i>${state.label} <b>${number.format(byState.get(state.key).length)}</b></span>`).join("");

  const sections = [];
  const review = byState.get("review");
  if (review.length > 0) {
    const rows = review.map((entry) => `<tr><td>${taskLink(entry)}</td><td>${esc(entry.title)}</td><td class="id"><a href="${esc(reviewing.get(entry.id).html_url)}">#${reviewing.get(entry.id).number}</a></td></tr>`);
    sections.push(`<h4>In review</h4>${shortTable(`<thead><tr><th>Task</th><th>Title</th><th>Pull request</th></tr></thead>`, rows, "more")}`);
  }
  const ready = byState.get("ready");
  if (ready.length > 0) {
    sections.push(`<h4>Ready now</h4>${shortTable(`<thead><tr><th>Task</th><th>Title</th></tr></thead>`, ready.map(taskRow), "more")}`);
  }
  const umbrellas = group.entries.filter((entry) => children.has(entry.id)).sort((a, b) => idNumber(a.id) - idNumber(b.id));
  if (umbrellas.length > 0) {
    const rows = umbrellas.map((entry) => {
      const parts = children.get(entry.id);
      const done = parts.filter((part) => part.status === "done").length;
      return `<tr><td>${taskLink(entry)}</td><td>${esc(entry.title)}</td><td class="num">${number.format(done)} of ${number.format(parts.length)}</td><td>${progressBar(done, parts.length)}</td></tr>`;
    });
    sections.push(`<h4>Umbrella tasks</h4>${shortTable(`<thead><tr><th>Task</th><th>Title</th><th class="num">Parts done</th><th>Progress</th></tr></thead>`, rows, "more")}`);
  }
  const waiting = byState.get("waiting");
  if (waiting.length > 0) {
    const needed = new Map();
    for (const entry of waiting) {
      for (const dep of entry.depends_on) {
        const other = byId.get(dep);
        if (other && other.status !== "done") {
          const code = other.milestone.split(" ")[0];
          needed.set(code, (needed.get(code) ?? new Set()).add(dep));
        }
      }
    }
    const rows = [...needed.entries()]
      .sort((a, b) => b[1].size - a[1].size)
      .map(([code, ids]) => `<tr><td>${esc(milestones.get(code).name)}</td><td class="num">${number.format(ids.size)}</td></tr>`);
    sections.push(`<h4>Waiting on</h4><p>The ${number.format(waiting.length)} waiting tasks here need these unmerged tasks first, counted by the milestone they belong to.</p>${shortTable(`<thead><tr><th>Milestone</th><th class="num">Unmerged tasks needed</th></tr></thead>`, rows, "more")}`);
  }
  const merged = merges.filter((merge) => merge.entry.milestone.split(" ")[0] === group.code).slice(0, 5);
  if (merged.length > 0) {
    const rows = merged.map((merge) => `<tr><td class="muted date">${esc(shortDate.format(merge.time))}</td><td>${taskLink(merge.entry)}</td><td>${esc(merge.entry.title)}</td></tr>`);
    sections.push(`<h4>Last merged</h4>${shortTable(`<thead><tr><th>Date</th><th>Task</th><th>Title</th></tr></thead>`, rows, "more")}`);
  }

  return `<details class="milestone" id="milestone-${esc(group.code)}">
    <summary><span class="count">${number.format(group.done)} of ${number.format(group.total)}</span>${esc(group.name)}<span class="states" aria-hidden="true">${bar}</span><span class="keys">${keys}</span></summary>
    <div class="milestone-body">${sections.join("")}</div>
  </details>`;
}

function renderMilestoneList(milestones, entries, byId, merges, reviewing) {
  const children = new Map();
  for (const entry of entries) {
    if (entry.parent) {
      const parts = children.get(entry.parent) ?? [];
      parts.push(entry);
      children.set(entry.parent, parts);
    }
  }
  document.getElementById("milestone-list").innerHTML = [...milestones.values()]
    .sort((a, b) => idNumber(a.code) - idNumber(b.code))
    .map((group) => renderMilestone(group, milestones, byId, children, merges, reviewing))
    .join("");
}

function deviationTask(task) {
  const items = task.log.deviations.map((deviation) => {
    const source = /^https:\/\//.test(deviation.link ?? "") ? `<p class="source"><a href="${esc(deviation.link)}">LiteLLM source</a></p>` : "";
    return `<li><p>${esc(deviation.what)}</p><p class="why">${esc(deviation.why)}</p>${source}</li>`;
  }).join("");
  const date = task.time === undefined ? "" : `<span class="date">${esc(shortDate.format(task.time))}</span>`;
  return `<details class="deviation-task"><summary><span class="count">${number.format(task.log.deviations.length)}</span>${date}${taskLink(task.entry)} ${esc(task.entry.title)}</summary><ul>${items}</ul></details>`;
}

// A deviation file lands in the same commit as its task, so the task's merge time dates it.
function renderDeviations(logs, byId, milestones, merges) {
  const latest = document.getElementById("deviation-latest");
  const list = document.getElementById("deviation-list");
  const mergedAt = new Map(merges.map((merge) => [merge.entry.id, merge.time]));
  const tasks = logs
    .filter((log) => log && log.task && (log.deviations ?? []).length > 0)
    .map((log) => ({ log, entry: byId.get(log.task) ?? { id: log.task, title: "", pr: null, milestone: "Not in the task list" }, time: mergedAt.get(log.task) }))
    .sort((a, b) => (b.time ?? 0) - (a.time ?? 0) || idNumber(a.entry.id) - idNumber(b.entry.id));
  const count = tasks.reduce((sum, task) => sum + task.log.deviations.length, 0);
  document.getElementById("deviation-count").textContent = `${plural(count, "deviation")} from ${plural(tasks.length, "task")}.`;
  if (tasks.length === 0) {
    latest.innerHTML = `<p class="empty">No task has logged a deviation yet.</p>`;
    list.innerHTML = latest.innerHTML;
    return;
  }
  latest.innerHTML = tasks.slice(0, 5).map(deviationTask).join("");

  const groups = new Map();
  for (const task of tasks) {
    const code = task.entry.milestone.split(" ")[0];
    const group = groups.get(code) ?? { code, name: milestones.get(code)?.name ?? task.entry.milestone, tasks: [], count: 0 };
    group.tasks.push(task);
    group.count += task.log.deviations.length;
    groups.set(code, group);
  }
  const ordered = [...groups.values()].sort((a, b) => idNumber(a.code) - idNumber(b.code));
  list.innerHTML = ordered.map((group) => {
    const rest = group.tasks.length <= 5 ? "" : `<details class="more"><summary>Show ${plural(group.tasks.length - 5, "more task")}</summary>${group.tasks.slice(5).map(deviationTask).join("")}</details>`;
    return `<details class="milestone deviation-group"><summary><span class="count">${plural(group.count, "deviation")} from ${plural(group.tasks.length, "task")}</span>${esc(group.name)}</summary><div class="milestone-body">${group.tasks.slice(0, 5).map(deviationTask).join("")}${rest}</div></details>`;
  }).join("") + `<p class="empty" id="deviation-none" hidden>No deviation matches the filter.</p>`;

  // The link is searched too, so a LiteLLM file path finds every deviation from that file.
  const listed = ordered.flatMap((group) => group.tasks);
  const rows = [...list.querySelectorAll(".deviation-task")].map((task, i) => ({
    task,
    head: `${listed[i].entry.id} ${listed[i].entry.title}`.toLowerCase(),
    items: [...task.querySelectorAll("li")].map((item, j) => {
      const deviation = listed[i].log.deviations[j];
      return { item, text: `${deviation.what} ${deviation.why} ${deviation.link ?? ""}`.toLowerCase() };
    }),
  }));
  document.getElementById("deviation-filter").addEventListener("input", (event) => {
    const query = event.target.value.trim().toLowerCase();
    let shown = 0;
    for (const row of rows) {
      const whole = row.head.includes(query);
      let matches = 0;
      for (const { item, text } of row.items) {
        item.hidden = !whole && !text.includes(query);
        matches += item.hidden ? 0 : 1;
      }
      row.task.hidden = matches === 0;
      row.task.open = query !== "" && matches > 0;
      shown += matches > 0 ? 1 : 0;
    }
    for (const holder of list.querySelectorAll(".deviation-group, .more")) {
      const matched = [...holder.querySelectorAll(".deviation-task")].some((task) => !task.hidden);
      holder.hidden = !matched;
      holder.open = query !== "" && matched;
    }
    document.getElementById("deviation-none").hidden = shown > 0;
  });
}

// Answers the open pull request of each task in review, or null when GitHub doesn't answer.
async function renderOpenPullRequests(byId) {
  const container = document.getElementById("open-prs");
  container.innerHTML = `<p class="empty">Asking GitHub</p>`;
  let pulls;
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/pulls?state=open&per_page=100`, { headers: { Accept: "application/vnd.github+json" }, signal: AbortSignal.timeout(10000) });
    if (!response.ok) {
      throw new Error(`GitHub answered HTTP ${response.status}`);
    }
    pulls = await response.json();
  } catch (error) {
    container.innerHTML = `<p class="empty">GitHub's API didn't answer (${esc(error.message)}). It allows 60 requests an hour per address without a login.</p>`;
    return null;
  }
  const reviewing = new Map();
  if (pulls.length === 0) {
    container.innerHTML = `<p class="empty">No pull request is open right now.</p>`;
    return reviewing;
  }
  const rows = pulls.map((pull) => {
    const task = /^task\/(B\d+)/.exec(pull.head.ref);
    const entry = task ? byId.get(task[1]) : undefined;
    if (entry) {
      reviewing.set(entry.id, pull);
    }
    const hours = Math.max(0, (Date.now() - Date.parse(pull.created_at)) / 3600000);
    const age = hours < 1 ? "under an hour" : hours < 48 ? `${Math.floor(hours)} h` : `${Math.floor(hours / 24)} days`;
    return `<tr><td class="id"><a href="${esc(pull.html_url)}">#${pull.number}</a></td><td>${esc(pull.title)}${entry ? ` <span class="muted">${esc(entry.milestone)}</span>` : ""}</td><td class="num muted">${age}</td></tr>`;
  }).join("");
  container.innerHTML = `<table><thead><tr><th>PR</th><th>Title</th><th class="num">Open for</th></tr></thead><tbody>${rows}</tbody></table>`;
  return reviewing;
}

// #milestone-M5 opens the Milestones tab at that milestone.
function showTab() {
  const links = [...document.querySelectorAll("nav a")];
  const tab = location.hash.startsWith("#milestone-") ? "#milestones" : location.hash;
  const current = links.find((link) => link.hash === tab) ?? links[0];
  for (const link of links) {
    document.getElementById(link.hash.slice(1)).hidden = link !== current;
    link.setAttribute("aria-current", String(link === current));
  }
  const milestone = tab === location.hash ? null : document.getElementById(location.hash.slice(1));
  if (milestone) {
    milestone.open = true;
    milestone.scrollIntoView();
  } else {
    window.scrollTo(0, 0);
  }
}

// A file counts as ported in the merged share of its covering tasks, counting only leaf tasks when it has any.
function coverage(entries, config, inventoryText) {
  const parents = new Set(entries.map((entry) => entry.parent).filter(Boolean));
  const excluded = (config.excluded ?? []).map((rule) => globToRegExp(rule.glob));
  const files = inventoryText.split("\n").filter((line) => line.endsWith(".py") && !excluded.some((rule) => rule.test(line)));
  const literal = new Map();
  const wild = [];
  for (const entry of entries) {
    if (entry.status === "done" && entry.resolution !== "merged") {
      continue;
    }
    for (const pattern of entry.covers ?? []) {
      const special = pattern.search(/[*?[{]/);
      if (special === -1) {
        literal.set(pattern, [...(literal.get(pattern) ?? []), entry]);
      } else {
        wild.push({ prefix: pattern.slice(0, special), rule: globToRegExp(pattern), entry });
      }
    }
  }
  const areas = AREAS.map((area) => ({ ...area, files: 0, full: 0, ported: 0 }));
  for (const file of files) {
    const owners = new Set(literal.get(file) ?? []);
    for (const glob of wild) {
      if (file.startsWith(glob.prefix) && glob.rule.test(file)) {
        owners.add(glob.entry);
      }
    }
    const leaves = [...owners].filter((entry) => !parents.has(entry.id));
    const counted = leaves.length > 0 ? leaves : [...owners];
    const share = counted.length === 0 ? 0 : counted.filter((entry) => entry.status === "done").length / counted.length;
    const area = areas.find((candidate) => candidate.prefixes.some((prefix) => file.startsWith(prefix)));
    area.files += 1;
    area.full += share === 1 ? 1 : 0;
    area.ported += share;
  }
  return areas;
}

// merges.txt is git log --first-parent on main, one "<unix time> <subject>" per line; a task's subject ends in (#<pr>).
function mergeTimes(byPull, mergesText) {
  const merges = [];
  for (const line of mergesText.split("\n")) {
    const match = /^(\d+) .*\(#(\d+)\)$/.exec(line);
    if (match && byPull.has(Number(match[2]))) {
      merges.push({ time: Number(match[1]) * 1000, entry: byPull.get(Number(match[2])) });
    }
  }
  return merges.sort((a, b) => b.time - a.time);
}

// Grouped by code, named by the most common spelling, since a few entries spell a milestone differently.
function milestoneGroups(entries) {
  const milestones = new Map();
  for (const entry of entries) {
    const code = entry.milestone.split(" ")[0];
    const group = milestones.get(code) ?? { code, names: new Map(), done: 0, total: 0, entries: [] };
    group.names.set(entry.milestone, (group.names.get(entry.milestone) ?? 0) + 1);
    group.entries.push(entry);
    group.total += 1;
    group.done += entry.status === "done" ? 1 : 0;
    milestones.set(code, group);
  }
  for (const group of milestones.values()) {
    group.name = [...group.names.entries()].sort((a, b) => b[1] - a[1])[0][0];
  }
  return milestones;
}

function renderMilestones(milestones, entries, done) {
  const rows = [...milestones.values()]
    .sort((a, b) => idNumber(a.code) - idNumber(b.code))
    .map((group) => `<tr><td><a href="#milestone-${esc(group.code)}">${esc(group.name)}</a></td><td class="num">${number.format(group.done)}</td><td class="num">${number.format(group.total)}</td><td>${progressBar(group.done, group.total)}</td></tr>`)
    .join("");
  document.getElementById("milestone-table").innerHTML = `<table><thead><tr><th>Milestone</th><th class="num">Done</th><th class="num">Tasks</th><th>Progress</th></tr></thead><tbody>${rows}</tbody><tfoot><tr><td>All</td><td class="num">${number.format(done)}</td><td class="num">${number.format(entries.length)}</td><td>${progressBar(done, entries.length)}</td></tr></tfoot></table>`;
}

function renderAreas(areas, files, full, ported) {
  const rows = areas.map((area) => `<tr><td>${esc(area.name)}</td><td class="num">${number.format(area.files)}</td><td class="num">${number.format(area.full)}</td><td>${progressBar(area.ported, area.files)}</td></tr>`).join("");
  document.getElementById("areas").innerHTML = `<table><thead><tr><th>Area</th><th class="num">Files</th><th class="num">Fully ported</th><th>Ported share</th></tr></thead><tbody>${rows}</tbody><tfoot><tr><td>All</td><td class="num">${number.format(files)}</td><td class="num">${number.format(full)}</td><td>${progressBar(ported, files)}</td></tr></tfoot></table>`;
}

function mergeRow(merge) {
  return `<tr><td class="muted date">${esc(shortDate.format(merge.time))}</td><td>${taskLink(merge.entry)}</td><td>${esc(merge.entry.title)}</td><td class="muted">${esc(merge.entry.milestone)}</td></tr>`;
}

function renderMerges(merges) {
  const container = document.getElementById("merges");
  if (merges.length === 0) {
    container.innerHTML = `<p class="empty">No task has merged yet.</p>`;
    return;
  }
  container.innerHTML = shortTable(`<thead><tr><th>Date</th><th>Task</th><th>Title</th><th>Milestone</th></tr></thead>`, merges.map(mergeRow), "older merges");
}

async function main() {
  const files = ["backlog.yaml", "deviations.yaml", "litellm-inventory.txt", "litellm.yaml", "merges.txt", "commit.txt", "prs.csv", "runs.csv", "agent-runs.csv", "targets.csv", "machine-boot.csv", "machine-hours.csv"];
  const [backlogText, deviationsText, inventoryText, configText, mergesText, commitText, prsText, runsText, agentText, targetsText, bootText, hoursText] = await Promise.all(files.map(fetchText));
  const entries = jsyaml.loadAll(backlogText).filter((entry) => entry && entry.id);
  const byId = new Map(entries.map((entry) => [entry.id, entry]));
  const byPull = new Map(entries.filter((entry) => entry.pr).map((entry) => [Number(entry.pr), entry]));
  const milestones = milestoneGroups(entries);
  const config = jsyaml.load(configText);
  const litellm = document.getElementById("litellm-link");
  litellm.href = `${config.repository}/tree/${config.commit}`;
  litellm.textContent = `LiteLLM ${config.version}`;

  const areas = coverage(entries, config, inventoryText);
  const fileCount = areas.reduce((sum, area) => sum + area.files, 0);
  const full = areas.reduce((sum, area) => sum + area.full, 0);
  const ported = areas.reduce((sum, area) => sum + area.ported, 0);
  const merges = mergeTimes(byPull, mergesText);
  const lastWeek = merges.filter((merge) => merge.time >= Date.now() - 7 * DAY).length;
  const done = entries.filter((entry) => entry.status === "done").length;
  const depths = new Map();
  const next = new Map();
  const paths = PATHS.map((path) => analyzePath(path, byId, depths, next));

  document.getElementById("figures").innerHTML = [
    figure(number.format(done), `of ${number.format(entries.length)}`, "Tasks done"),
    figure(percent(ported, fileCount), `of ${number.format(fileCount)} files`, "LiteLLM's Python ported"),
    figure(number.format(lastWeek), `${(lastWeek / 7).toFixed(1)} a day`, "Merged in the last 7 days"),
    figure(number.format(paths[0].chain.length), "tasks", `Longest open chain: ${PATHS[0].name}`),
  ].join("");
  renderMilestones(milestones, entries, done);
  renderAreas(areas, fileCount, full, ported);

  const found = new Set(merges.map((merge) => merge.entry.id));
  const unmatched = entries.filter((entry) => entry.resolution === "merged" && !found.has(entry.id)).map((entry) => entry.id);
  if (unmatched.length > 0) {
    const note = document.getElementById("unmatched");
    note.textContent = `Not shown: ${unmatched.join(", ")}. Their commits on main don't name a pull request, so their merge day is unknown.`;
    note.hidden = false;
  }
  renderHistory({
    pulls: parseCsv(prsText),
    runs: parseCsv(runsText),
    agentRuns: parseCsv(agentText),
    targets: parseCsv(targetsText),
    boot: parseCsv(bootText)[0] ?? null,
    hours: parseCsv(hoursText),
    merges,
    milestones,
    byPull,
    entries,
  });
  renderMerges(merges);
  renderDeviations(jsyaml.loadAll(deviationsText), byId, milestones, merges);

  const [sha, committed] = commitText.trim().split(" ");
  document.getElementById("built").innerHTML = `Built from <a class="id" href="${GITHUB}/commit/${esc(sha)}">${esc(sha.slice(0, 8))}</a>, committed ${esc(longDate.format(Number(committed) * 1000))}.`;
  document.getElementById("status").hidden = true;

  const reviewing = await renderOpenPullRequests(byId);
  document.getElementById("review-note").hidden = reviewing !== null;
  document.getElementById("paths").innerHTML = paths.map((facts) => renderPath(facts, byId, reviewing ?? new Map())).join("");
  renderMilestoneList(milestones, entries, byId, merges, reviewing ?? new Map());
  if (location.hash.startsWith("#milestone-")) {
    showTab();
  }
}

showTab();
window.addEventListener("hashchange", showTab);
main().catch((error) => {
  const status = document.getElementById("status");
  status.classList.add("error");
  status.textContent = `The page couldn't load its data: ${error.message}`;
});
