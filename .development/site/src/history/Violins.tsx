import { memo, useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from "react";
import { compact, duration, gigabytes, number, plural, rank, sum } from "../format.ts";
import { STAGE_TICKS } from "./charts.ts";
import { CI_EVENTS, STAGE_COLORS, STEPS, TOTAL, type History } from "./facts.ts";

export type ViolinMode = "time" | "size" | "ci" | "agents" | "disk";

export const VIOLIN_ABOUT: Record<ViolinMode, string> = {
  time: "Time per task pull request in each step. A step under a minute counts as one minute.",
  size: "Lines changed, files changed and commits per task pull request.",
  ci: "Wall time of the gate runs that passed or failed, by event. Cancelled runs are left out, and a run under a minute counts as one minute.",
  agents: "Wall time of each worker and reviewer run, from launch to exit, and the output tokens of each worker run.",
  disk: "Size of a task checkout's build folder when it was deleted after the task merged.",
};

const VIOLIN = { lane: 34, label: 64, axis: 18, points: 80, gap: 44 };
const CI_TICKS = [1, 2, 5, 10, 20, 30, 60, 120, 240];
const COUNT_TICKS = [1, 3, 10, 30, 100, 300, 1000, 3000, 10000, 30000];
const TIP_MARKS = [
  { name: "25th", at: 0.25 },
  { name: "median", at: 0.5 },
  { name: "75th", at: 0.75 },
  { name: "90th", at: 0.9 },
];
const SIZES: { key: "lines" | "files" | "commits"; title: string }[] = [
  { key: "lines", title: "Lines changed" },
  { key: "files", title: "Files changed" },
  { key: "commits", title: "Commits" },
];

interface Lane {
  label: string;
  color: string;
  values: number[];
  zeros: number;
}

interface Metric {
  title: string;
  ticks: number[];
  format: (value: number) => string;
  lanes: Lane[];
}

interface Scale {
  lo: number;
  hi: number;
  left: number;
  right: number;
}

interface Tip {
  text: string;
  x: number;
  y: number;
}

// A lane keeps its values sorted, so its percentiles are read by rank.
function stageLanes(facts: History, select: (stage: number) => number[]): Lane[] {
  return facts.live.map((period) => {
    const all = select(period.key);
    return { label: period.label, color: STAGE_COLORS[period.key], values: all.filter((value) => value > 0).sort((a, b) => a - b), zeros: all.filter((value) => value <= 0).length };
  });
}

function wholeMinutes(value: number): number {
  return Math.max(1, value);
}

export function violinMetrics(facts: History, mode: ViolinMode): Metric[] {
  if (mode === "time") {
    return [...STEPS, TOTAL].map((step) => ({
      title: step.label === "Total" ? "Total, first commit to merged" : `${step.label} step`,
      ticks: STAGE_TICKS,
      format: duration,
      lanes: stageLanes(facts, (stage) =>
        facts.taskRows
          .filter((row) => row.stage === stage)
          .map((row) => row[step.key])
          .filter((value) => value !== null)
          .map(wholeMinutes),
      ),
    }));
  }
  if (mode === "size") {
    return SIZES.map((size) => ({
      title: size.title,
      ticks: COUNT_TICKS,
      format: (value: number) => number.format(value),
      lanes: stageLanes(facts, (stage) => facts.taskRows.filter((row) => row.stage === stage).map((row) => row[size.key])),
    }));
  }
  if (mode === "ci") {
    return CI_EVENTS.map((event) => ({
      title: `${event.label} runs`,
      ticks: CI_TICKS,
      format: duration,
      lanes: stageLanes(facts, (stage) => facts.runs.filter((run) => run.event === event.key && run.stage === stage && run.conclusion !== "cancelled").map((run) => wholeMinutes((run.updated - run.started) / 60))),
    }));
  }
  if (mode === "agents") {
    return [
      { title: "Worker run time", ticks: STAGE_TICKS, format: duration, lanes: stageLanes(facts, (stage) => facts.agentRuns.filter((run) => run.role === "w" && run.stage === stage).map((run) => wholeMinutes((run.end - run.start) / 60))) },
      {
        title: "Reviewer run time",
        ticks: STAGE_TICKS,
        format: duration,
        lanes: stageLanes(facts, (stage) => facts.agentRuns.filter((run) => run.role === "r" && run.stage === stage).map((run) => wholeMinutes((run.end - run.start) / 60))),
      },
      {
        title: "Output tokens per worker run",
        ticks: [1000, 3000, 10000, 30000, 100000, 300000, 1000000],
        format: (value: number) => compact.format(value),
        lanes: stageLanes(facts, (stage) =>
          facts.agentRuns
            .filter((run) => run.role === "w" && run.stage === stage)
            .map((run) => run.output_tokens)
            .filter((tokens) => tokens !== null),
        ),
      },
    ];
  }
  return [{ title: "Build folder per task checkout", ticks: [0.1, 0.3, 1, 3, 10, 30], format: gigabytes, lanes: stageLanes(facts, (stage) => facts.targets.filter((target) => target.stage === stage).map((target) => target.gb)) }];
}

function violinX(scale: Scale, log: number): number {
  return scale.left + ((log - scale.lo) / (scale.hi - scale.lo)) * (scale.right - scale.left);
}

function violinTip(lane: Lane, metric: Metric): string {
  const lines = [`${lane.label}, ${metric.title}`, `${plural(lane.values.length, "value")}${lane.zeros > 0 ? `, ${number.format(lane.zeros)} zeros left out` : ""}`];
  for (const mark of TIP_MARKS) {
    lines.push(`${mark.name.padEnd(7)} ${metric.format(rank(lane.values, mark.at))}`);
  }
  return lines.join("\n");
}

function ViolinLane({ lane, index, metric, scale, width }: { lane: Lane; index: number; metric: Metric; scale: Scale; width: number }) {
  const top = index * VIOLIN.lane;
  const mid = top + VIOLIN.lane / 2;
  const name = (
    <text className="lane-name" x="0" y={mid} dominantBaseline="middle">
      {lane.label}
    </text>
  );
  if (lane.values.length === 0) {
    return (
      <>
        {name}
        <text x={scale.left} y={mid} dominantBaseline="middle">
          none yet
        </text>
      </>
    );
  }
  const logs = lane.values.map(Math.log10);
  const average = sum(logs) / logs.length;
  const spread = Math.sqrt(sum(logs.map((value) => (value - average) ** 2)) / logs.length);
  const bandwidth = Math.max(0.04, 1.06 * spread * logs.length ** -0.2);
  const density: { x: number; total: number }[] = [];
  for (let i = 0; i < VIOLIN.points; i++) {
    const at = scale.lo + (i / (VIOLIN.points - 1)) * (scale.hi - scale.lo);
    let total = 0;
    for (const value of logs) {
      const z = (at - value) / bandwidth;
      total += Math.exp(-0.5 * z * z);
    }
    density.push({ x: violinX(scale, at), total });
  }
  const peak = Math.max(...density.map((point) => point.total));
  const half = VIOLIN.lane / 2 - 3;
  const upper = density.map((point) => `${point.x.toFixed(1)},${(mid - (half * point.total) / peak).toFixed(1)}`);
  const lower = density.map((point) => `${point.x.toFixed(1)},${(mid + (half * point.total) / peak).toFixed(1)}`).reverse();
  const [x25, x50, x75] = [0.25, 0.5, 0.75].map((at) => violinX(scale, Math.log10(rank(lane.values, at))));
  return (
    <>
      {name}
      <path d={`M${upper.join("L")}L${lower.join("L")}Z`} style={{ fill: `var(${lane.color})`, stroke: `var(${lane.color})` }} fillOpacity="0.4" strokeWidth="1" />
      <rect className="box" x={x25.toFixed(1)} y={mid - 3} width={Math.max(2, x75 - x25).toFixed(1)} height="6" />
      <line className="median" x1={x50.toFixed(1)} x2={x50.toFixed(1)} y1={mid - 3} y2={mid + 3} />
      <rect className="hit" x="0" y={top} width={width} height={VIOLIN.lane} data-tip={violinTip(lane, metric)} />
    </>
  );
}

function ViolinBlock({ metric, width }: { metric: Metric; width: number }) {
  const all = metric.lanes.flatMap((lane) => lane.values);
  if (all.length === 0) {
    return (
      <div className="violin">
        <p>{metric.title}</p>
        <p className="empty">No values yet.</p>
      </div>
    );
  }
  const low = Math.log10(Math.min(...all));
  const high = Math.log10(Math.max(...all));
  const pad = Math.max(0.05, (high - low) * 0.04);
  const scale = { lo: low - pad, hi: high + pad, left: VIOLIN.label, right: width - 8 };
  const height = metric.lanes.length * VIOLIN.lane + VIOLIN.axis;
  const grid: { tick: number; x: number }[] = [];
  let previous = -Infinity;
  for (const tick of metric.ticks.filter((value) => Math.log10(value) >= scale.lo && Math.log10(value) <= scale.hi)) {
    const x = violinX(scale, Math.log10(tick));
    if (x - previous >= VIOLIN.gap) {
      grid.push({ tick, x });
      previous = x;
    }
  }
  return (
    <div className="violin">
      <p>{metric.title}</p>
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={metric.title}>
        {grid.map((line) => (
          <g key={line.tick}>
            <line className="grid" x1={line.x.toFixed(1)} x2={line.x.toFixed(1)} y1="0" y2={height - VIOLIN.axis} />
            <text x={line.x.toFixed(1)} y={height - 4} textAnchor="middle">
              {metric.format(line.tick)}
            </text>
          </g>
        ))}
        {metric.lanes.map((lane, i) => (
          <ViolinLane key={lane.label} lane={lane} index={i} metric={metric} scale={scale} width={width} />
        ))}
      </svg>
    </div>
  );
}

const ViolinBlocks = memo(function ViolinBlocks({ metrics, width }: { metrics: Metric[]; width: number }) {
  return metrics.map((metric) => <ViolinBlock key={metric.title} metric={metric} width={width} />);
});

export function ViolinTable({ metrics }: { metrics: Metric[] }) {
  return (
    <div className="table-scroll" id="violin-table">
      <table>
        <thead>
          <tr>
            <th>Values</th>
            <th>Stage</th>
            <th className="num">Count</th>
            <th className="num">10th</th>
            <th className="num">25th</th>
            <th className="num">Median</th>
            <th className="num">75th</th>
            <th className="num">90th</th>
          </tr>
        </thead>
        <tbody>
          {metrics.flatMap((metric) =>
            metric.lanes.map((lane) => (
              <tr key={`${metric.title} ${lane.label}`}>
                <td>{metric.title}</td>
                <td>{lane.label}</td>
                <td className="num">{number.format(lane.values.length)}</td>
                {[0.1, 0.25, 0.5, 0.75, 0.9].map((at) => (
                  <td key={at} className="num">
                    {lane.values.length === 0 ? "" : metric.format(rank(lane.values, at))}
                  </td>
                ))}
              </tr>
            )),
          )}
        </tbody>
      </table>
    </div>
  );
}

export function Violins({ metrics }: { metrics: Metric[] }) {
  const holder = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [tip, setTip] = useState<Tip | null>(null);

  // A hidden tab has no width, so the shapes are drawn when the tab shows, and again when the width changes.
  useEffect(() => {
    const container = holder.current;
    if (container === null) {
      return;
    }
    const resizer = new ResizeObserver((records) => {
      setWidth(Math.floor(records[0].contentRect.width));
    });
    resizer.observe(container);
    return () => {
      resizer.disconnect();
    };
  }, []);

  useLayoutEffect(() => {
    const element = box.current;
    if (element === null || tip === null) {
      return;
    }
    const x = Math.max(8, Math.min(tip.x + 14, window.innerWidth - element.offsetWidth - 8));
    const y = tip.y + 14 + element.offsetHeight > window.innerHeight ? tip.y - element.offsetHeight - 10 : tip.y + 14;
    element.style.left = `${x}px`;
    element.style.top = `${y}px`;
  }, [tip]);

  function showTip(event: PointerEvent): void {
    const hit = event.target instanceof Element ? event.target.closest<SVGElement>(".hit") : null;
    const text = hit?.dataset.tip;
    setTip(text === undefined ? null : { text, x: event.clientX, y: event.clientY });
  }

  return (
    <div
      className="violins"
      id="violins"
      ref={holder}
      onPointerMove={showTip}
      onPointerDown={showTip}
      onPointerLeave={() => {
        setTip(null);
      }}
    >
      {width > 0 && <ViolinBlocks metrics={metrics} width={width} />}
      <div className="tip" hidden={tip === null} ref={box}>
        {tip?.text}
      </div>
    </div>
  );
}
