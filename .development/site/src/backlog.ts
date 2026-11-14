import { idNumber } from "./format.ts";
import type { Config, Entry } from "./schema.ts";

export type TaskState = "done" | "review" | "ready" | "waiting" | "unplanned";

export const STATES: { key: TaskState; label: string }[] = [
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

export interface RequestPath {
  name: string;
  about: string;
  targets: string[];
}

export const PATHS: RequestPath[] = [
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

export interface Milestone {
  code: string;
  name: string;
  names: Map<string, number>;
  done: number;
  total: number;
  entries: Entry[];
}

export interface Merge {
  time: number;
  entry: Entry;
}

export interface Area {
  name: string;
  files: number;
  full: number;
  ported: number;
}

export interface PathFacts {
  path: RequestPath;
  total: number;
  done: number;
  open: Entry[];
  chain: Entry[];
  withoutFile: number;
}

// R milestones hold cleanup and refactoring, so the porting figures leave them out (Max, 2026-11-13).
export function porting(entry: Entry): boolean {
  return !entry.milestone.startsWith("R");
}

// The M milestones in number order, then the R ones.
export function milestoneOrder(a: { code: string }, b: { code: string }): number {
  return Number(a.code.startsWith("R")) - Number(b.code.startsWith("R")) || idNumber(a.code) - idNumber(b.code);
}

export function codeOf(entry: { milestone: string }): string {
  return entry.milestone.split(" ")[0];
}

export function taskOf(byId: Map<string, Entry>, id: string): Entry {
  const entry = byId.get(id);
  if (entry === undefined) {
    throw new Error(`${id} is not in the task list`);
  }
  return entry;
}

export function groupOf(milestones: Map<string, Milestone>, entry: Entry): Milestone {
  const group = milestones.get(codeOf(entry));
  if (group === undefined) {
    throw new Error(`${entry.id} has no milestone group`);
  }
  return group;
}

// Globs follow globset with literal_separator: * and ? stay inside one path segment, ** crosses them.
export function globToRegExp(glob: string): RegExp {
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
    } else if (c === "[" && glob.includes("]", i + 2)) {
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

function depthOf(id: string, byId: Map<string, Entry>, depths: Map<string, number>, next: Map<string, string>): number {
  const known = depths.get(id);
  if (known !== undefined) {
    return known;
  }
  depths.set(id, 0);
  const entry = taskOf(byId, id);
  let best = 0;
  for (const dep of entry.depends_on) {
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
export function analyzePath(path: RequestPath, byId: Map<string, Entry>, depths: Map<string, number>, next: Map<string, string>): PathFacts {
  const members = new Set<string>();
  const entries: Entry[] = [];
  const stack = [...path.targets];
  for (let id = stack.pop(); id !== undefined; id = stack.pop()) {
    const entry = byId.get(id);
    if (members.has(id) || entry === undefined) {
      continue;
    }
    members.add(id);
    entries.push(entry);
    stack.push(...entry.depends_on);
  }
  const open = entries.filter((entry) => entry.status !== "done").sort((a, b) => idNumber(a.id) - idNumber(b.id));

  let head = path.targets[0];
  for (const id of path.targets) {
    if (depthOf(id, byId, depths, next) > depthOf(head, byId, depths, next)) {
      head = id;
    }
  }
  const chain: Entry[] = [];
  for (let id: string | undefined = head; id !== undefined; id = next.get(id)) {
    const entry = taskOf(byId, id);
    if (entry.status !== "done") {
      chain.push(entry);
    }
  }
  chain.reverse();
  const counted = entries.filter(porting);
  return { path, total: counted.length, done: counted.filter((entry) => entry.status === "done").length, open, chain, withoutFile: open.filter((entry) => entry.file === null).length };
}

export function taskState(entry: Entry, byId: Map<string, Entry>, reviewing: ReadonlyMap<string, unknown>): TaskState {
  if (entry.status === "done") {
    return "done";
  }
  if (reviewing.has(entry.id)) {
    return "review";
  }
  if (entry.file === null) {
    return "unplanned";
  }
  return entry.depends_on.some((dep) => byId.has(dep) && byId.get(dep)?.status !== "done") ? "waiting" : "ready";
}

// A file counts as ported in the merged share of its covering tasks, counting only leaf tasks when it has any.
export function coverage(entries: Entry[], config: Config, inventoryText: string): Area[] {
  const parents = new Set(entries.map((entry) => entry.parent));
  const excluded = config.excluded.map((rule) => globToRegExp(rule.glob));
  const files = inventoryText.split("\n").filter((line) => line.endsWith(".py") && !excluded.some((rule) => rule.test(line)));
  const literal = new Map<string, Entry[]>();
  const wild: { prefix: string; rule: RegExp; entry: Entry }[] = [];
  for (const entry of entries) {
    if (entry.status === "done" && entry.resolution !== "merged") {
      continue;
    }
    for (const pattern of entry.covers) {
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
    const part = counted.length === 0 ? 0 : counted.filter((entry) => entry.status === "done").length / counted.length;
    for (const area of areas) {
      if (area.prefixes.some((prefix) => file.startsWith(prefix))) {
        area.files += 1;
        area.full += part === 1 ? 1 : 0;
        area.ported += part;
        break;
      }
    }
  }
  return areas;
}

// merges.txt is git log --first-parent on main, one "<unix time> <subject>" per line; a task's subject ends in (#<pr>).
export function mergeTimes(byPull: Map<number, Entry>, mergesText: string): Merge[] {
  const merges: Merge[] = [];
  for (const line of mergesText.split("\n")) {
    const match = /^(\d+) .*\(#(\d+)\)$/.exec(line);
    const entry = match === null ? undefined : byPull.get(Number(match[2]));
    if (match !== null && entry !== undefined) {
      merges.push({ time: Number(match[1]) * 1000, entry });
    }
  }
  return merges.sort((a, b) => b.time - a.time);
}

// Grouped by code, named by the most common spelling, since a few entries spell a milestone differently.
export function milestoneGroups(entries: Entry[]): Map<string, Milestone> {
  const milestones = new Map<string, Milestone>();
  for (const entry of entries) {
    const code = codeOf(entry);
    const group = milestones.get(code) ?? { code, name: entry.milestone, names: new Map<string, number>(), done: 0, total: 0, entries: [] };
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
