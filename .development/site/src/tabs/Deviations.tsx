import { useMemo, useState } from "react";
import { codeOf, milestoneOrder, type Merge, type Milestone } from "../backlog.ts";
import { idNumber, number, plural, shortDate } from "../format.ts";
import { TaskLink } from "../parts.tsx";
import type { DeviationLog, Entry } from "../schema.ts";
import type { Site } from "../site.ts";

interface DeviationTask {
  log: DeviationLog;
  entry: Pick<Entry, "id" | "title" | "pr" | "milestone">;
  time: number | null;
  head: string;
  texts: string[];
}

interface DeviationGroup {
  code: string;
  name: string;
  tasks: DeviationTask[];
  count: number;
}

// A deviation file lands in the same commit as its task, so the task's merge time dates it.
function deviationTasks(logs: DeviationLog[], byId: Map<string, Entry>, merges: Merge[]): DeviationTask[] {
  const mergedAt = new Map(merges.map((merge) => [merge.entry.id, merge.time]));
  const tasks = logs
    .filter((log) => log.deviations.length > 0)
    .map((log) => {
      const entry = byId.get(log.task) ?? { id: log.task, title: "", pr: null, milestone: "Not in the task list" };
      return {
        log,
        entry,
        time: mergedAt.get(log.task) ?? null,
        head: `${entry.id} ${entry.title}`.toLowerCase(),
        // The link is searched too, so a LiteLLM file path finds every deviation from that file.
        texts: log.deviations.map((deviation) => `${deviation.what} ${deviation.why} ${deviation.link}`.toLowerCase()),
      };
    });
  return tasks.sort((a, b) => (b.time ?? 0) - (a.time ?? 0) || idNumber(a.entry.id) - idNumber(b.entry.id));
}

function deviationGroups(tasks: DeviationTask[], milestones: Map<string, Milestone>): DeviationGroup[] {
  const groups = new Map<string, DeviationGroup>();
  for (const task of tasks) {
    const code = codeOf(task.entry);
    const group = groups.get(code) ?? { code, name: milestones.get(code)?.name ?? task.entry.milestone, tasks: [], count: 0 };
    group.tasks.push(task);
    group.count += task.log.deviations.length;
    groups.set(code, group);
  }
  return [...groups.values()].sort(milestoneOrder);
}

function matches(task: DeviationTask, query: string): boolean {
  return task.head.includes(query) || task.texts.some((text) => text.includes(query));
}

function TaskBlock({ task, query }: { task: DeviationTask; query: string | null }) {
  const whole = query === null || task.head.includes(query);
  const shown = query === null || matches(task, query);
  return (
    <details className="deviation-task" hidden={!shown} open={query !== null && query !== "" && shown}>
      <summary>
        <span className="count">{number.format(task.log.deviations.length)}</span>
        {task.time !== null && <span className="date">{shortDate.format(task.time)}</span>}
        <TaskLink entry={task.entry} /> {task.entry.title}
      </summary>
      <ul>
        {task.log.deviations.map((deviation, i) => (
          <li key={i} hidden={!whole && !task.texts[i].includes(query)}>
            <p>{deviation.what}</p>
            <p className="why">{deviation.why}</p>
            {deviation.link.startsWith("https://") && (
              <p className="source">
                <a href={deviation.link}>LiteLLM source</a>
              </p>
            )}
          </li>
        ))}
      </ul>
    </details>
  );
}

function GroupBlock({ group, query }: { group: DeviationGroup; query: string }) {
  const rest = group.tasks.slice(5);
  const matched = group.tasks.some((task) => matches(task, query));
  const restMatched = rest.some((task) => matches(task, query));
  return (
    <details className="milestone deviation-group" hidden={!matched} open={query !== "" && matched}>
      <summary>
        <span className="count">
          {plural(group.count, "deviation")} from {plural(group.tasks.length, "task")}
        </span>
        {group.name}
      </summary>
      <div className="milestone-body">
        {group.tasks.slice(0, 5).map((task) => (
          <TaskBlock key={task.entry.id} task={task} query={query} />
        ))}
        {rest.length > 0 && (
          <details className="more" hidden={!restMatched} open={query !== "" && restMatched}>
            <summary>Show {plural(rest.length, "more task")}</summary>
            {rest.map((task) => (
              <TaskBlock key={task.entry.id} task={task} query={query} />
            ))}
          </details>
        )}
      </div>
    </details>
  );
}

export function Deviations({ site, hidden }: { site: Site; hidden: boolean }) {
  const [filter, setFilter] = useState("");
  const tasks = useMemo(() => deviationTasks(site.logs, site.byId, site.merges), [site]);
  const groups = useMemo(() => deviationGroups(tasks, site.milestones), [tasks, site]);
  const query = filter.trim().toLowerCase();
  const count = tasks.reduce((total, task) => total + task.log.deviations.length, 0);
  const none = <p className="empty">No task has logged a deviation yet.</p>;
  return (
    <section id="deviations" hidden={hidden}>
      <div className="section-head">
        <h2>Deviations</h2>
        <p>Where betterlite differs from LiteLLM on purpose. Each task logs every such place, with the reason and the LiteLLM code it departs from.</p>
      </div>
      <div className="block">
        <header>
          <h3>Latest</h3>
          <p>The last five tasks that merged with deviations, newest first.</p>
        </header>
        <div id="deviation-latest">{tasks.length === 0 ? none : tasks.slice(0, 5).map((task) => <TaskBlock key={task.entry.id} task={task} query={null} />)}</div>
      </div>
      <div className="block">
        <header>
          <h3>By milestone</h3>
          <p>Every deviation, grouped by the milestone of the task that logged it. Inside a milestone the newest task comes first.</p>
          <p id="deviation-count">
            {plural(count, "deviation")} from {plural(tasks.length, "task")}.
          </p>
          <label className="filter">
            Filter
            <input
              id="deviation-filter"
              type="search"
              placeholder="Task, word or LiteLLM file"
              autoComplete="off"
              spellCheck={false}
              value={filter}
              onChange={(event) => {
                setFilter(event.target.value);
              }}
            />
          </label>
        </header>
        <div id="deviation-list">
          {tasks.length === 0 ? none : groups.map((group) => <GroupBlock key={group.code} group={group} query={query} />)}
          {tasks.length > 0 && !tasks.some((task) => matches(task, query)) && (
            <p className="empty" id="deviation-none">
              No deviation matches the filter.
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
