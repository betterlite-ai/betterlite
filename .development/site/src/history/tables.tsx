import { Fragment } from "react";
import { milestoneOrder, porting } from "../backlog.ts";
import { codeOf } from "../backlog.ts";
import { compact, DAY, dayTime, duration, gigabytes, GITHUB, longDate, mean, number, oneDecimal, quantile, rank, share, shortDate, sum, yearDate } from "../format.ts";
import { Figure, Swatch } from "../parts.tsx";
import { hourValues, MEDIAN_WINDOW, slotName, type HourMode } from "./charts.ts";
import { CI_EVENTS, CUTOVER_PRS, inStage, spanDays, STAGE_COLORS, STEPS, TOTAL, countPerDay, type History, type Measured, type Period, type PeriodStats, type StageRow, type StepKey } from "./facts.ts";

interface Measure {
  label: string;
  key: keyof PeriodStats;
  format: (value: number) => string;
}

const FIGURES: Measure[] = [
  { label: "Tasks merged a day", key: "tasksADay", format: (value) => oneDecimal.format(value) },
  { label: "Median time from first commit to merged, task PRs", key: "taskMedian", format: duration },
  { label: "CI runs per merged PR", key: "ciPerPr", format: (value) => oneDecimal.format(value) },
  { label: "SSD written a day", key: "ssd", format: gigabytes },
];

const MEASURES: Measure[] = [
  { label: "Days", key: "length", format: (value) => oneDecimal.format(value) },
  { label: "Tasks merged", key: "tasks", format: (value) => number.format(value) },
  { label: "Tasks merged a day", key: "tasksADay", format: (value) => oneDecimal.format(value) },
  { label: "PRs merged", key: "prs", format: (value) => number.format(value) },
  { label: "PRs merged a day", key: "prsADay", format: (value) => oneDecimal.format(value) },
  { label: "Task PR, first commit to merged, median", key: "taskMedian", format: duration },
  { label: "Task PR, first commit to merged, 90th percentile", key: "task90", format: duration },
  { label: "Build step, median", key: "build", format: duration },
  { label: "Checks step, median", key: "checks", format: duration },
  { label: "Queue step, median", key: "queue", format: duration },
  { label: "Lines changed per task PR, median", key: "lines", format: (value) => number.format(value) },
  { label: "Merge queue tries per task PR", key: "queues", format: (value) => oneDecimal.format(value) },
  { label: "CI runs per merged PR", key: "ciPerPr", format: (value) => oneDecimal.format(value) },
  { label: "CI runs that failed", key: "failed", format: share },
  { label: "CI runs cancelled", key: "cancelled", format: share },
  { label: "Agent runs", key: "agentRuns", format: (value) => number.format(value) },
  { label: "Agent hours", key: "agentHours", format: (value) => number.format(Math.round(value)) },
  { label: "Output tokens per merged task", key: "tokensPerTask", format: (value) => compact.format(value) },
  { label: "Claude worker time in tests and builds", key: "heavy", format: share },
  { label: "Claude worker time waiting on the model", key: "model", format: share },
  { label: "Output tokens a second of model wait", key: "speed", format: (value) => number.format(Math.round(value)) },
  { label: "Build folder per task checkout, median", key: "target", format: gigabytes },
  { label: "SSD written a day", key: "ssd", format: gigabytes },
];

const SIZE_BUCKETS = [
  { label: "Under 100", from: 0, to: 100 },
  { label: "100 to 499", from: 100, to: 500 },
  { label: "500 to 1,999", from: 500, to: 2000 },
  { label: "2,000 to 4,999", from: 2000, to: 5000 },
  { label: "5,000 and up", from: 5000, to: Infinity },
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

export function GroupHead({ first, periods, names }: { first: string; periods: Period[]; names: string[] }) {
  return (
    <thead>
      <tr>
        <th rowSpan={2}>{first}</th>
        {periods.map((period) => (
          <th key={period.key} className="group" colSpan={names.length}>
            <StageLabel period={period} />
          </th>
        ))}
      </tr>
      <tr>
        {periods.map((period) =>
          names.map((name, i) => (
            <th key={`${period.key} ${name}`} className={i === 0 ? "num group" : "num"}>
              {name}
            </th>
          )),
        )}
      </tr>
    </thead>
  );
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
        CI runs count every gate run on the pull request and in the merge queue, cancelled ones included. Claude worker time and model speed come from Claude Code transcripts only.
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

function DailyRow({ history, period, taskCounts, prCounts }: { history: History; period: Period; taskCounts: number[]; prCounts: number[] }) {
  const days = history.days;
  const slots = days.map((day, i) => i).filter((i) => days[i] / 1000 >= period.from && days[i] / 1000 + 86400 <= period.to);
  if (slots.length === 0) {
    return (
      <tr>
        <td>
          <StageLabel period={period} />
        </td>
        <td className="num">0</td>
        <td className="num muted" colSpan={5}>
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
      <td className="num">{oneDecimal.format(sum(slots.map((i) => prCounts[i])) / slots.length)}</td>
    </tr>
  );
}

export function DailyTable({ history }: { history: History }) {
  const taskCounts = countPerDay(
    history.days,
    history.tasks.map((merge) => merge.time),
    null,
  );
  const prCounts = countPerDay(
    history.days,
    history.rows.map((row) => row.merged * 1000),
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
            <th className="num">All PRs a day</th>
          </tr>
        </thead>
        <tbody>
          {history.live.map((period) => (
            <DailyRow key={period.key} history={history} period={period} taskCounts={taskCounts} prCounts={prCounts} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function HoursTable({ history, mode }: { history: History; mode: HourMode }) {
  const values = history.live.map((period) => hourValues(history, period, mode));
  const slots = [...Array(mode === "hour" ? 24 : 7).keys()];
  return (
    <div className="table-scroll" id="hours-table">
      <table>
        <thead>
          <tr>
            <th>{mode === "hour" ? "Hour" : "Weekday"}</th>
            {history.live.map((period) => (
              <th key={period.key} className="num">
                <StageLabel period={period} />, merges a day
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {slots.map((slot) => (
            <tr key={slot}>
              <td>{slotName(mode, slot)}</td>
              {history.live.map((period, i) => (
                <td key={period.key} className="num">
                  {values[i][slot].toFixed(2)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function StageCount({ history, rows }: { history: History; rows: StageRow[] }) {
  const counted = `${number.format(rows.length)} merged since ${shortDate.format(rows[0].merged * 1000)}`;
  const parts = history.live.map((period) => `${number.format(rows.filter((row) => row.stage === period.key).length)} ${period.key === 1 ? "in" : "started in"} stage ${period.key}`);
  const crossed = number.format(rows.filter((row) => row.stage === 0).length);
  return <p id="stage-count">{history.cutovers.length === 0 ? `${counted}.` : `${counted}: ${parts.join(", ")} and ${crossed} started before ${history.cutovers.length === 1 ? "the switch" : "a switch"} and merged after it.`}</p>;
}

// The chart's legend: the pull request under the cursor, or the latest one, with each step's time, its median line and what the step covers.
export function StageLegend({ rows, medians, point, hidden, onToggle }: { rows: StageRow[]; medians: Record<StepKey, (number | null)[]>; point: number; hidden: StepKey[]; onToggle: (step: StepKey) => void }) {
  const row = rows[point];
  return (
    <div className="table-scroll legend" id="stage-legend">
      <table>
        <caption>
          Pull request #{row.number}
          {row.task !== null && ` ${row.task}`}, merged {dayTime.format(row.merged * 1000)}
        </caption>
        <thead>
          <tr>
            <th>Step</th>
            <th className="num">This pull request</th>
            <th className="num">Median of last {MEDIAN_WINDOW}</th>
            <th>From and to</th>
          </tr>
        </thead>
        <tbody>
          {STEPS.map((step) => {
            const shown = !hidden.includes(step.key);
            return (
              <tr key={step.key} className={shown ? undefined : "off"}>
                <td>
                  <button
                    type="button"
                    aria-pressed={shown}
                    title={`${shown ? "Hide" : "Show"} ${step.label} on the chart`}
                    onClick={() => {
                      onToggle(step.key);
                    }}
                  >
                    <Swatch color={step.color} />
                    {step.label}
                  </button>
                </td>
                <td className="num">
                  <Cell value={row[step.key]} format={duration} missing="none" />
                </td>
                <td className="num">
                  <Cell value={medians[step.key][point]} format={duration} missing="none" />
                </td>
                <td className="muted">{step.about}</td>
              </tr>
            );
          })}
        </tbody>
        <tfoot>
          <tr>
            <td>
              <i className="swatch" aria-hidden="true" />
              {TOTAL.label}
            </td>
            <td className="num">
              <Cell value={row.total} format={duration} missing="none" />
            </td>
            <td className="num">
              <Cell value={medians.total[point]} format={duration} missing="none" />
            </td>
            <td className="muted">{TOTAL.about}</td>
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

export function StageTable({ history, rows }: { history: History; rows: StageRow[] }) {
  const lines = [...STEPS, TOTAL].map((step) => (
    <tr key={step.key}>
      <td>{step.label}</td>
      {history.columns.map((period) => {
        const inside = rows.filter((row) => inStage(period, row.stage)).map((row) => row[step.key]);
        return (
          <Fragment key={period.key}>
            <td className="num group">
              <Cell value={quantile(inside, 0.5)} format={duration} missing="none yet" />
            </td>
            <td className="num">
              <Cell value={quantile(inside, 0.9)} format={duration} missing="none yet" />
            </td>
          </Fragment>
        );
      })}
    </tr>
  ));
  return (
    <div className="table-scroll" id="stage-table">
      <table>
        <thead>
          <tr>
            <th rowSpan={2}>Step</th>
            {history.columns.map((period) => (
              <th key={period.key} className="group" colSpan={2}>
                <StageLabel period={period} />
              </th>
            ))}
          </tr>
          <tr>
            {history.columns.map((period) => (
              <Fragment key={period.key}>
                <th className="num group">Median</th>
                <th className="num">90th</th>
              </Fragment>
            ))}
          </tr>
        </thead>
        <tbody>{lines.slice(0, -1)}</tbody>
        <tfoot>{lines.at(-1)}</tfoot>
      </table>
    </div>
  );
}

export function Sizes({ history }: { history: History }) {
  return (
    <div className="table-scroll" id="sizes">
      <table>
        <GroupHead first="Lines changed" periods={history.live} names={["PRs", "Median time", "CI runs each"]} />
        <tbody>
          {SIZE_BUCKETS.map((bucket) => (
            <tr key={bucket.label}>
              <td>{bucket.label}</td>
              {history.live.map((period) => {
                const inside = history.taskRows.filter((row) => row.stage === period.key && row.lines >= bucket.from && row.lines < bucket.to);
                return (
                  <Fragment key={period.key}>
                    <td className="num group">{number.format(inside.length)}</td>
                    <td className="num">
                      <Cell
                        value={quantile(
                          inside.map((row) => row.total),
                          0.5,
                        )}
                        format={duration}
                        missing="none"
                      />
                    </td>
                    <td className="num">
                      <Cell value={mean(inside.map((row) => row.runs))} format={(value) => oneDecimal.format(value)} missing="none" />
                    </td>
                  </Fragment>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function CiTable({ history }: { history: History }) {
  return (
    <div className="table-scroll" id="ci-table">
      <table>
        <GroupHead first="Event" periods={history.live} names={["Runs", "Failed", "Median time"]} />
        <tbody>
          {CI_EVENTS.map((event) => (
            <tr key={event.key}>
              <td>{event.label}</td>
              {history.live.map((period) => {
                const runs = history.runs.filter((run) => run.event === event.key && run.stage === period.key);
                const finished = runs.filter((run) => run.conclusion !== "cancelled").map((run) => (run.updated - run.started) / 60);
                const failed = runs.length === 0 ? null : runs.filter((run) => run.conclusion === "failure").length / runs.length;
                return (
                  <Fragment key={period.key}>
                    <td className="num group">{number.format(runs.length)}</td>
                    <td className="num">
                      <Cell value={failed} format={share} missing="none" />
                    </td>
                    <td className="num">
                      <Cell value={quantile(finished, 0.5)} format={duration} missing="none" />
                    </td>
                  </Fragment>
                );
              })}
            </tr>
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
