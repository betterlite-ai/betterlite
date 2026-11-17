import { Fragment } from "react";
import { milestoneOrder, porting } from "../backlog.ts";
import { codeOf } from "../backlog.ts";
import { compact, DAY, gigabytes, GITHUB, longDate, number, oneDecimal, rank, share, shortDate, sum, yearDate } from "../format.ts";
import { Figure, Swatch } from "../parts.tsx";
import { CUTOVER_PRS, spanDays, STAGE_COLORS, countPerDay, type History, type Measured, type Period, type PeriodStats } from "./facts.ts";

interface Measure {
  label: string;
  key: keyof PeriodStats;
  format: (value: number) => string;
}

const FIGURES: Measure[] = [
  { label: "Tasks merged a day", key: "tasksADay", format: (value) => oneDecimal.format(value) },
  { label: "SSD written a day", key: "ssd", format: gigabytes },
];

const MEASURES: Measure[] = [
  { label: "Days", key: "length", format: (value) => oneDecimal.format(value) },
  { label: "Tasks merged", key: "tasks", format: (value) => number.format(value) },
  { label: "Tasks merged a day", key: "tasksADay", format: (value) => oneDecimal.format(value) },
  { label: "Agent runs", key: "agentRuns", format: (value) => number.format(value) },
  { label: "Agent hours", key: "agentHours", format: (value) => number.format(Math.round(value)) },
  { label: "Output tokens per merged task", key: "tokensPerTask", format: (value) => compact.format(value) },
  { label: "Claude worker time in tests and builds", key: "heavy", format: share },
  { label: "Claude worker time waiting on the model", key: "model", format: share },
  { label: "Output tokens a second of model wait", key: "speed", format: (value) => number.format(Math.round(value)) },
  { label: "Build folder per task checkout, median", key: "target", format: gigabytes },
  { label: "SSD written a day", key: "ssd", format: gigabytes },
];

export function StageLabel({ period }: { period: Period }) {
  if (period.key === 0) {
    return period.label;
  }
  return (
    <>
      <Swatch color={STAGE_COLORS[period.key]} />
      {period.label}
    </>
  );
}

export function Cell({ value, format, missing }: { value: number | null; format: (value: number) => string; missing: string }) {
  return value === null || Number.isNaN(value) ? <span className="muted">{missing}</span> : format(value);
}

export function StageNote({ cutovers }: { cutovers: number[] }) {
  const links = CUTOVER_PRS.map((pull) => (
    <a key={pull} href={`${GITHUB}/pull/${pull}`}>
      #{pull}
    </a>
  ));
  if (cutovers.length === 0) {
    return <p id="stage-note">Stage 2 starts when {links[0]} merges. Until then every number here belongs to stage 1.</p>;
  }
  return (
    <p id="stage-note">
      {cutovers.map((cutover, i) => (
        <Fragment key={cutover}>
          Stage {i + 1} ran until {links[i]} merged on {longDate.format(cutover * 1000)}.{" "}
        </Fragment>
      ))}
      Stage {cutovers.length + 1} is everything after it.
    </p>
  );
}

function StageValue({ stage, value }: { stage: number; value: string | null }) {
  return (
    <>
      <Swatch color={STAGE_COLORS[stage]} />
      <span className="sr-only">stage {stage}: </span>
      {value ?? <span className="muted">no data yet</span>}
    </>
  );
}

export function StageFigures({ live }: { live: Measured[] }) {
  const latest = live[live.length - 1];
  return (
    <div className="figures small" id="stage-figures">
      <div className="keys">
        {live.map((period) => (
          <span key={period.key}>
            <Swatch color={STAGE_COLORS[period.key]} />
            Stage {period.key}
          </span>
        ))}
      </div>
      {FIGURES.map((item) => {
        const value = latest.stats[item.key];
        const missing = latest.key === 1 ? "none" : null;
        return (
          <Figure
            key={item.key}
            label={item.label}
            value={<StageValue stage={latest.key} value={value === null ? missing : item.format(value)} />}
            small={live.slice(0, -1).map((period, i) => {
              const earlier = period.stats[item.key];
              return (
                <Fragment key={period.key}>
                  {i > 0 && " "}
                  <StageValue stage={period.key} value={earlier === null ? "none" : item.format(earlier)} />
                </Fragment>
              );
            })}
          />
        );
      })}
    </div>
  );
}

export function Compare({ history }: { history: History }) {
  return (
    <div className="table-scroll" id="stage-compare">
      <table>
        <thead>
          <tr>
            <th>Measure</th>
            {history.columns.map((period) => (
              <th key={period.key} className="num">
                <StageLabel period={period} />
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {MEASURES.map((measure) => (
            <tr key={measure.label}>
              <td>{measure.label}</td>
              {history.columns.map((period) => (
                <td key={period.key} className="num">
                  <Cell value={period.stats[measure.key]} format={measure.format} missing="none yet" />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="table-note">
        Claude worker time and model speed come from Claude Code transcripts only.
        {history.boot !== null && ` Stage 1 SSD writes start from the laptop's own counter since ${yearDate.format(history.boot.from * 1000)} and add the 5-minute samples from ${yearDate.format(history.boot.to * 1000)} on.`}
      </p>
    </div>
  );
}

export function Pace({ history }: { history: History }) {
  const open = history.entries.filter((entry) => porting(entry) && entry.status !== "done").length;
  const week = history.tasks.filter((merge) => merge.time >= history.now * 1000 - 7 * DAY).length;
  const paces = [...history.live.map((period) => ({ key: period.label, label: <StageLabel period={period} />, length: period.stats.length, count: period.stats.tasks })), { key: "week", label: "Last 7 days", length: 7, count: week }];
  return (
    <div className="table-scroll" id="pace">
      <table>
        <thead>
          <tr>
            <th>Pace</th>
            <th className="num">Days</th>
            <th className="num">Tasks merged</th>
            <th className="num">A day</th>
            <th className="num">{number.format(open)} open tasks done by</th>
          </tr>
        </thead>
        <tbody>
          {paces.map((pace) => {
            const rate = pace.length > 0 ? pace.count / pace.length : 0;
            return (
              <tr key={pace.key}>
                <td>{pace.label}</td>
                <td className="num">{oneDecimal.format(pace.length)}</td>
                <td className="num">{number.format(pace.count)}</td>
                <td className="num">{oneDecimal.format(rate)}</td>
                <td className="num">{rate > 0 ? yearDate.format(history.now * 1000 + (open / rate) * DAY) : "never at this pace"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function DailyRow({ history, period, taskCounts }: { history: History; period: Period; taskCounts: number[] }) {
  const days = history.days;
  const slots = days.map((day, i) => i).filter((i) => days[i] / 1000 >= period.from && days[i] / 1000 + 86400 <= period.to);
  if (slots.length === 0) {
    return (
      <tr>
        <td>
          <StageLabel period={period} />
        </td>
        <td className="num">0</td>
        <td className="num muted" colSpan={4}>
          no whole day yet
        </td>
      </tr>
    );
  }
  const counts = slots.map((i) => taskCounts[i]);
  const busiest = slots.reduce((best, i) => (taskCounts[i] > taskCounts[best] ? i : best), slots[0]);
  return (
    <tr>
      <td>
        <StageLabel period={period} />
      </td>
      <td className="num">{number.format(slots.length)}</td>
      <td className="num">{oneDecimal.format(sum(counts) / slots.length)}</td>
      <td className="num">
        {number.format(
          rank(
            counts.toSorted((a, b) => a - b),
            0.5,
          ),
        )}
      </td>
      <td className="num">
        {number.format(taskCounts[busiest])} on {shortDate.format(days[busiest])}
      </td>
      <td className="num">{number.format(counts.filter((count) => count === 0).length)}</td>
    </tr>
  );
}

export function DailyTable({ history }: { history: History }) {
  const taskCounts = countPerDay(
    history.days,
    history.tasks.map((merge) => merge.time),
    null,
  );
  return (
    <div className="table-scroll" id="daily-table">
      <table>
        <thead>
          <tr>
            <th>Stage</th>
            <th className="num">Whole days</th>
            <th className="num">Tasks a day</th>
            <th className="num">Median day</th>
            <th className="num">Busiest day</th>
            <th className="num">Days with none</th>
          </tr>
        </thead>
        <tbody>
          {history.live.map((period) => (
            <DailyRow key={period.key} history={history} period={period} taskCounts={taskCounts} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function MilestonePace({ history }: { history: History }) {
  const week = history.now * 1000 - 7 * DAY;
  return (
    <div className="table-scroll" id="milestone-pace">
      <table>
        <thead>
          <tr>
            <th>Milestone</th>
            <th className="num">Open</th>
            {history.live.map((period) => (
              <th key={period.key} className="num">
                <StageLabel period={period} /> a day
              </th>
            ))}
            <th className="num">Last 7 days</th>
            <th className="num">Days left</th>
          </tr>
        </thead>
        <tbody>
          {[...history.milestones.values()].sort(milestoneOrder).map((group) => {
            const merged = history.tasks.filter((merge) => codeOf(merge.entry) === group.code);
            const open = group.total - group.done;
            const recent = merged.filter((merge) => merge.time >= week).length;
            return (
              <tr key={group.code}>
                <td>
                  <a href={`#milestone-${group.code}`}>{group.name}</a>
                </td>
                <td className="num">{number.format(open)}</td>
                {history.live.map((period) => {
                  const length = spanDays(period);
                  const count = merged.filter((merge) => merge.stage === period.key).length;
                  return (
                    <td key={period.key} className="num">
                      {length > 0 ? oneDecimal.format(count / length) : "0.0"}
                    </td>
                  );
                })}
                <td className="num">{number.format(recent)}</td>
                <td className="num">{open === 0 ? "done" : recent === 0 ? "no merges this week" : `${number.format(Math.ceil(open / (recent / 7)))} days`}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
