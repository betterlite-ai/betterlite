import { Fragment } from "react";
import { codeOf, groupOf, milestoneOrder, STATES, taskOf, taskState, type Milestone, type PathFacts, type TaskState } from "../backlog.ts";
import { idNumber, number, shortDate } from "../format.ts";
import { MilestonePace } from "../history/tables.tsx";
import { ProgressBar, ShortTable, TaskLink } from "../parts.tsx";
import type { Entry, OpenPull } from "../schema.ts";
import type { Site } from "../site.ts";

type Reviewing = Map<string, OpenPull>;

function PathBlock({ facts, byId, reviewing }: { facts: PathFacts; byId: Map<string, Entry>; reviewing: Reviewing }) {
  const { path, total, done, open, chain, withoutFile } = facts;
  const ready = open.filter((entry) => taskState(entry, byId, reviewing) === "ready");
  const inReview = open.filter((entry) => reviewing.has(entry.id)).length;
  const head = (
    <thead>
      <tr>
        <th>Task</th>
        <th>Title</th>
        <th>Milestone</th>
      </tr>
    </thead>
  );
  const readyRows = ready.map((entry) => (
    <tr key={entry.id}>
      <td>
        <TaskLink entry={entry} />
      </td>
      <td>{entry.title}</td>
      <td className="muted">{entry.milestone}</td>
    </tr>
  ));
  return (
    <div className="path">
      <h4>{path.name}</h4>
      <p>
        {path.about} Done when{" "}
        {path.targets.map((id, i) => (
          <Fragment key={id}>
            {i > 0 && ", "}
            <TaskLink entry={taskOf(byId, id)} />
          </Fragment>
        ))}{" "}
        merge.
      </p>
      <ProgressBar part={done} whole={total} />
      <p>
        {number.format(done)} of {number.format(total)} tasks merged, dependencies included. The longest open chain is {number.format(chain.length)} tasks, {number.format(withoutFile)} still need a task file and {number.format(inReview)}{" "}
        {inReview === 1 ? "is" : "are"} in review. These {number.format(ready.length)} are ready to start:
      </p>
      {ready.length === 0 ? <p className="empty">Nothing on this path can start yet.</p> : <ShortTable head={head} rows={readyRows} rest="more" />}
      {chain.length > 0 && (
        <details>
          <summary>Longest open chain, {number.format(chain.length)} tasks, first step first</summary>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th className="num">Step</th>
                  <th>Task</th>
                  <th>Title</th>
                  <th>Milestone</th>
                </tr>
              </thead>
              <tbody>
                {chain.map((entry, i) => (
                  <tr key={entry.id}>
                    <td className="num muted">{i + 1}</td>
                    <td>
                      <TaskLink entry={entry} />
                    </td>
                    <td>{entry.title}</td>
                    <td className="muted">{entry.milestone}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      )}
    </div>
  );
}

function InReview({ entries, reviewing }: { entries: Entry[]; reviewing: Reviewing }) {
  const head = (
    <thead>
      <tr>
        <th>Task</th>
        <th>Title</th>
        <th>Pull request</th>
      </tr>
    </thead>
  );
  const rows = entries.map((entry) => {
    const pull = reviewing.get(entry.id);
    return (
      <tr key={entry.id}>
        <td>
          <TaskLink entry={entry} />
        </td>
        <td>{entry.title}</td>
        <td className="id">{pull !== undefined && <a href={pull.html_url}>#{pull.number}</a>}</td>
      </tr>
    );
  });
  return (
    <>
      <h4>In review</h4>
      <ShortTable head={head} rows={rows} rest="more" />
    </>
  );
}

function ReadyNow({ entries }: { entries: Entry[] }) {
  const head = (
    <thead>
      <tr>
        <th>Task</th>
        <th>Title</th>
      </tr>
    </thead>
  );
  const rows = entries.map((entry) => (
    <tr key={entry.id}>
      <td>
        <TaskLink entry={entry} />
      </td>
      <td>{entry.title}</td>
    </tr>
  ));
  return (
    <>
      <h4>Ready now</h4>
      <ShortTable head={head} rows={rows} rest="more" />
    </>
  );
}

function Umbrellas({ entries, partsOf }: { entries: Entry[]; partsOf: Map<string, Entry[]> }) {
  const head = (
    <thead>
      <tr>
        <th>Task</th>
        <th>Title</th>
        <th className="num">Parts done</th>
        <th>Progress</th>
      </tr>
    </thead>
  );
  const rows = entries.map((entry) => {
    const parts = partsOf.get(entry.id) ?? [];
    const done = parts.filter((part) => part.status === "done").length;
    return (
      <tr key={entry.id}>
        <td>
          <TaskLink entry={entry} />
        </td>
        <td>{entry.title}</td>
        <td className="num">
          {number.format(done)} of {number.format(parts.length)}
        </td>
        <td>
          <ProgressBar part={done} whole={parts.length} />
        </td>
      </tr>
    );
  });
  return (
    <>
      <h4>Umbrella tasks</h4>
      <ShortTable head={head} rows={rows} rest="more" />
    </>
  );
}

function WaitingOn({ waiting, site }: { waiting: Entry[]; site: Site }) {
  const needed = new Map<Milestone, Set<string>>();
  for (const entry of waiting) {
    for (const dep of entry.depends_on) {
      const other = site.byId.get(dep);
      if (other !== undefined && other.status !== "done") {
        const group = groupOf(site.milestones, other);
        needed.set(group, (needed.get(group) ?? new Set<string>()).add(dep));
      }
    }
  }
  const head = (
    <thead>
      <tr>
        <th>Milestone</th>
        <th className="num">Unmerged tasks needed</th>
      </tr>
    </thead>
  );
  const rows = [...needed.entries()]
    .sort((a, b) => b[1].size - a[1].size)
    .map(([group, ids]) => (
      <tr key={group.code}>
        <td>{group.name}</td>
        <td className="num">{number.format(ids.size)}</td>
      </tr>
    ));
  return (
    <>
      <h4>Waiting on</h4>
      <p>The {number.format(waiting.length)} waiting tasks here need these unmerged tasks first, counted by the milestone they belong to.</p>
      <ShortTable head={head} rows={rows} rest="more" />
    </>
  );
}

function LastMerged({ group, site }: { group: Milestone; site: Site }) {
  const merged = site.merges.filter((merge) => codeOf(merge.entry) === group.code).slice(0, 5);
  if (merged.length === 0) {
    return null;
  }
  const head = (
    <thead>
      <tr>
        <th>Date</th>
        <th>Task</th>
        <th>Title</th>
      </tr>
    </thead>
  );
  const rows = merged.map((merge) => (
    <tr key={`${merge.entry.id} ${merge.time}`}>
      <td className="muted date">{shortDate.format(merge.time)}</td>
      <td>
        <TaskLink entry={merge.entry} />
      </td>
      <td>{merge.entry.title}</td>
    </tr>
  ));
  return (
    <>
      <h4>Last merged</h4>
      <ShortTable head={head} rows={rows} rest="more" />
    </>
  );
}

function MilestoneBlock({ group, site, partsOf, reviewing }: { group: Milestone; site: Site; partsOf: Map<string, Entry[]>; reviewing: Reviewing }) {
  const byState: Record<TaskState, Entry[]> = { done: [], review: [], ready: [], waiting: [], unplanned: [] };
  const ordered = group.entries.toSorted((a, b) => idNumber(a.id) - idNumber(b.id));
  for (const entry of ordered) {
    byState[taskState(entry, site.byId, reviewing)].push(entry);
  }
  const umbrellas = ordered.filter((entry) => partsOf.has(entry.id));
  return (
    <details className="milestone" id={`milestone-${group.code}`}>
      <summary>
        <span className="count">
          {number.format(group.done)} of {number.format(group.total)}
        </span>
        {group.name}
        <span className="states" aria-hidden="true">
          {STATES.filter((state) => byState[state.key].length > 0).map((state) => (
            <span key={state.key} className={`state-${state.key}`} style={{ flexGrow: byState[state.key].length }} title={`${state.label}: ${number.format(byState[state.key].length)}`} />
          ))}
        </span>
        <span className="keys">
          {STATES.map((state) => (
            <span key={state.key}>
              <i className={`swatch state-${state.key}`} aria-hidden="true" />
              {state.label} <b>{number.format(byState[state.key].length)}</b>
            </span>
          ))}
        </span>
      </summary>
      <div className="milestone-body">
        {byState.review.length > 0 && <InReview entries={byState.review} reviewing={reviewing} />}
        {byState.ready.length > 0 && <ReadyNow entries={byState.ready} />}
        {umbrellas.length > 0 && <Umbrellas entries={umbrellas} partsOf={partsOf} />}
        {byState.waiting.length > 0 && <WaitingOn waiting={byState.waiting} site={site} />}
        <LastMerged group={group} site={site} />
      </div>
    </details>
  );
}

function MilestoneList({ site, reviewing }: { site: Site; reviewing: Reviewing }) {
  const partsOf = new Map<string, Entry[]>();
  for (const entry of site.entries) {
    if (entry.parent !== null) {
      partsOf.set(entry.parent, [...(partsOf.get(entry.parent) ?? []), entry]);
    }
  }
  return [...site.milestones.values()].sort(milestoneOrder).map((group) => <MilestoneBlock key={group.code} group={group} site={site} partsOf={partsOf} reviewing={reviewing} />);
}

export function Milestones({ site, reviewing, answered, hidden }: { site: Site; reviewing: Reviewing | null; answered: boolean; hidden: boolean }) {
  const asking = <p className="empty">Asking GitHub which tasks are in review</p>;
  return (
    <section id="milestones" hidden={hidden}>
      <div className="section-head">
        <h2>Milestones</h2>
        <p>Where every task stands, by milestone. Open a milestone to see what is in review, what can start now, its umbrella tasks and what it waits on.</p>
      </div>
      <div className="block">
        <header>
          <h3>Request paths</h3>
          <p>Two requests that should work end to end. Each one counts every task its target tasks need through their dependencies.</p>
        </header>
        <div className="paths" id="paths">
          {reviewing === null ? asking : site.paths.map((facts) => <PathBlock key={facts.path.name} facts={facts} byId={site.byId} reviewing={reviewing} />)}
        </div>
      </div>
      <div className="block">
        <header>
          <h3>Pace</h3>
          <p>Open tasks in each milestone and how many merged in the last 7 days. Days left assumes that pace holds and the milestone gets no new tasks.</p>
        </header>
        <MilestonePace history={site.history} />
      </div>
      <div className="block">
        <header>
          <h3>By milestone</h3>
          <p>
            Done: merged, or closed without code. In review: a task pull request is open. Ready: the task file is written and every dependency is merged. Waiting: a dependency is not merged yet. No task file: the planner has not written it
            yet.
          </p>
          {reviewing !== null && !answered && <p id="review-note">GitHub didn't answer, so no task shows as in review.</p>}
        </header>
        <div id="milestone-list">{reviewing === null ? asking : <MilestoneList site={site} reviewing={reviewing} />}</div>
      </div>
    </section>
  );
}
