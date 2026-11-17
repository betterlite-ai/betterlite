import { useCallback, useMemo, useState } from "react";
import { number, oneDecimal } from "../format.ts";
import { Modes, type Mode } from "../parts.tsx";
import { LatestMerges, OpenPulls } from "../pulls.tsx";
import type { Review, Site } from "../site.ts";
import { Chart } from "./Chart.tsx";
import { drawDaily, drawTotals } from "./charts.ts";
import { type History as Facts, type Series, type ViewKey } from "./facts.ts";
import { MachineCharts, MachineNote, MachineTable } from "./Machine.tsx";
import { Compare, DailyTable, Pace, StageFigures, StageNote } from "./tables.tsx";
import { VIOLIN_ABOUT, violinMetrics, Violins, ViolinTable, type ViolinMode } from "./Violins.tsx";
import { Wait } from "./Wait.tsx";

type AgentMode = "runs" | "hours";

const VIEW_MODES: Mode<ViewKey>[] = [
  { value: "tasks", label: "Tasks" },
  { value: "milestones", label: "Milestones" },
];
const VIOLIN_MODES: Mode<ViolinMode>[] = [
  { value: "agents", label: "Agents" },
  { value: "disk", label: "Disk" },
];
const AGENT_MODES: Mode<AgentMode>[] = [
  { value: "runs", label: "Runs" },
  { value: "hours", label: "Hours" },
];

function agentView(facts: Facts, mode: AgentMode): Series[] {
  const counts = new Map<string, number>();
  for (const run of facts.agentRuns) {
    counts.set(run.model, (counts.get(run.model) ?? 0) + 1);
  }
  const top = [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 6)
    .map(([model]) => model)
    .sort();
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

function Burnup({ history, unmatched }: { history: Facts; unmatched: string[] }) {
  const [mode, setMode] = useState<ViewKey>("tasks");
  const draw = useCallback((container: HTMLElement) => drawTotals(container, history.days, history.views[mode], history.cutovers), [history, mode]);
  return (
    <div className="block">
      <header>
        <h3>Tasks merged</h3>
        <p>Total merged into main at the end of each day. The table says when the open tasks would be done at each pace, if the task list stopped growing.</p>
        <Modes label="Tasks merged by" modes={VIEW_MODES} value={mode} onChange={setMode} />
        {unmatched.length > 0 && <p id="unmatched">Not shown: {unmatched.join(", ")}. Their commits on main don't name a pull request, so their merge day is unknown.</p>}
      </header>
      <div>
        <Chart id="burnup" draw={draw} />
        <Pace history={history} />
      </div>
    </div>
  );
}

function Daily({ history }: { history: Facts }) {
  const [mode, setMode] = useState<ViewKey>("tasks");
  const draw = useCallback((container: HTMLElement) => drawDaily(container, history.days, history.views[mode], history.cutovers, (value) => number.format(value)), [history, mode]);
  return (
    <div className="block">
      <header>
        <h3>Merges per day</h3>
        <p>Pull requests merged into main each day. The line is the average of that day and the six before it. The table counts whole days only.</p>
        <Modes label="Merges per day by" modes={VIEW_MODES} value={mode} onChange={setMode} />
      </header>
      <div>
        <Chart id="daily" draw={draw} />
        <DailyTable history={history} />
      </div>
    </div>
  );
}

function Distributions({ history }: { history: Facts }) {
  const [mode, setMode] = useState<ViolinMode>("agents");
  const metrics = useMemo(() => violinMetrics(history, mode), [history, mode]);
  return (
    <div className="block">
      <header>
        <h3>Distributions</h3>
        <p>Each shape shows how the values spread on a logarithmic scale, wide where many of them sit. The dark bar runs from the 25th to the 75th percentile and the light tick marks the median. Hover or tap a row for its numbers.</p>
        <Modes label="Distributions of" modes={VIOLIN_MODES} value={mode} onChange={setMode} />
        <p id="violin-about">{VIOLIN_ABOUT[mode]}</p>
      </header>
      <div>
        <Violins metrics={metrics} />
        <details>
          <summary>Show the numbers</summary>
          <ViolinTable metrics={metrics} />
        </details>
      </div>
    </div>
  );
}

function AgentRuns({ history }: { history: Facts }) {
  const [mode, setMode] = useState<AgentMode>("runs");
  const draw = useCallback((container: HTMLElement) => drawDaily(container, history.days, agentView(history, mode), history.cutovers, (value) => (mode === "hours" ? oneDecimal.format(value) : number.format(value))), [history, mode]);
  return (
    <div className="block">
      <header>
        <h3>Agent runs</h3>
        <p>Worker and reviewer runs started each day, by model.</p>
        <Modes label="Agent runs as" modes={AGENT_MODES} value={mode} onChange={setMode} />
      </header>
      <div>
        <Chart id="agents" draw={draw} />
      </div>
    </div>
  );
}

export function History({ site, review, hidden }: { site: Site; review: Review; hidden: boolean }) {
  const history = site.history;
  return (
    <section id="history" hidden={hidden}>
      <div className="section-head">
        <h2>History</h2>
        <StageNote cutovers={history.cutovers} />
        <p>Hover a chart to read its values below it. Click a legend entry to hide that line, drag across a chart to zoom in and double-click to zoom out.</p>
      </div>
      {history.days.length === 0 ? (
        <p className="empty">No pull request has merged yet.</p>
      ) : (
        <>
          <div className="block">
            <header>
              <h3>Three stages</h3>
              <p>Counts per day go by merge time.</p>
            </header>
            <div>
              <div className="stage-about">
                <p>
                  Stage 1, until #722 merged. A worker agent wrote the task and ran its whole check list on the laptop before the pull request opened: the build, the task's tests, the oracle cases that compare answers with LiteLLM, and the
                  mutation tests. Every task checkout kept its own build folder, several gigabytes each. With dozens of agents at once the builds filled the disk, wrote to it all day and slowed every agent down. CI then checked lint on the
                  pull request and ran the task's tests again in the merge queue.
                </p>
                <p>
                  Stage 2, from #722 until #1544. On the laptop a worker only formats and lints the code and runs the checks that need no test build. The tests, the oracle cases and the mutation tests run in CI on the pull request. A red
                  run goes back to the same worker with the failing lines, the reviewer checks the fix, and after three red rounds the task is marked blocked. The merge queue still tests the merged result before it reaches main.
                </p>
                <p>
                  Stage 3, from #1544 on. The laptop only runs the agents. A worker formats its code and runs the checks that build nothing, and clippy moves to CI with the tests, the oracle cases and the mutation tests. On a pull request
                  CI runs lint first and starts the other parts once it passes, so a compile error costs one short job. A red run goes back to the same worker, up to eight rounds. With no builds on the laptop, memory and the vendors' seats
                  decide how many tasks run at once, and DeepSeek went from 32 seats to 128.
                </p>
                <p>
                  What stage 2 should change: less disk traffic and less waiting on the laptop, so pull requests open sooner. What it costs: more CI runs, and some tasks need an extra round when CI finds what the laptop no longer checks.
                </p>
                <p>What stage 3 should change: many more tasks at once, since builds no longer fill the laptop's disk and CPU. What it costs: more CI rounds per task, since a compile error now shows up in CI instead of on the laptop.</p>
              </div>
              <StageFigures live={history.live} />
              <Compare history={history} />
            </div>
          </div>
          <Burnup history={history} unmatched={site.unmatched} />
          <Daily history={history} />
          <Distributions history={history} />
          <AgentRuns history={history} />
          <div className="block">
            <header>
              <h3>Where a worker's time goes</h3>
              <p>
                Each Claude worker run split by what it waited on: the model's replies, compaction (the model summarizing a long session to free room), the commands it ran and its other tools. The tables split the reply time by reply size
                and by model.
              </p>
            </header>
            <Wait history={history} />
          </div>
          <div className="block">
            <header>
              <h3>Build machine</h3>
              <p>
                One laptop runs every agent. Until stage 3 it also ran every build and lint. The table puts the averages since the machine started next to each stage's 5-minute samples and the last full hour. The dashed line in each chart
                is the average since it started.
              </p>
              <MachineNote history={history} />
            </header>
            <div>
              <MachineTable history={history} />
              <MachineCharts history={history} />
            </div>
          </div>
        </>
      )}
      <div className="block">
        <header>
          <h3>Open pull requests</h3>
          <p>Read live from GitHub when the page loads.</p>
        </header>
        <OpenPulls review={review} byId={site.byId} />
      </div>
      <div className="block">
        <header>
          <h3>Latest merges</h3>
          <p>Newest first.</p>
        </header>
        <LatestMerges merges={site.merges} />
      </div>
    </section>
  );
}
