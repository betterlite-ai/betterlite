import uPlot from "uplot";
import { cssColor, dayTime, hourOnly, mean, number, oneDecimal, shortDate, sum } from "../format.ts";
import { countPerDay, STAGE_COLORS, type History, type HourKey, type Series } from "./facts.ts";

// Gridlines of the logarithmic time scales, in minutes.
export const STAGE_TICKS = [1, 5, 15, 60, 240, 1440, 4320, 10080];
// Steps between date ticks, in seconds: whole days on the daily charts, hours too on the others.
const DAY_STEPS = [86400, 172800, 604800, 1209600, 2419200];
const HOUR_STEPS = [3600, 10800, 21600, 43200, ...DAY_STEPS];
// uPlot's own text for a legend value it has none for.
const NO_VALUE = "--";

export interface MachineChart {
  title: string;
  unit: string;
  key: HourKey;
}

// Without the cursor on the chart, the legend shows the last point instead of dashes.
function legendAtEnd(chart: uPlot): void {
  if (chart.cursor.idx == null) {
    chart.setLegend({ idx: chart.data[0].length - 1 });
  }
}

function paintStageBands(chart: uPlot, cutovers: number[]): void {
  const { left, top, width, height } = chart.bbox;
  for (const [i, cutover] of cutovers.entries()) {
    const from = Math.max(left, chart.valToPos(cutover, "x", true));
    const to = i + 1 < cutovers.length ? Math.min(left + width, chart.valToPos(cutovers[i + 1], "x", true)) : left + width;
    if (from < to) {
      chart.ctx.save();
      chart.ctx.fillStyle = `${cssColor(STAGE_COLORS[i + 2])}14`;
      chart.ctx.fillRect(from, top, to - from, height);
      chart.ctx.restore();
    }
  }
}

function paintStageLines(chart: uPlot, cutovers: number[]): void {
  const { left, top, width, height } = chart.bbox;
  for (const [i, cutover] of cutovers.entries()) {
    const x = Math.round(chart.valToPos(cutover, "x", true));
    if (x < left || x > left + width) {
      continue;
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
    // Each later label sits one line lower, so labels of close cutovers don't overlap.
    ctx.fillText(`stage ${i + 2}`, x + (flip ? -4 : 4) * ratio, top + (4 + 14 * i) * ratio);
    ctx.restore();
  }
}

function axis(): uPlot.Axis {
  return {
    stroke: cssColor("--muted"),
    font: "11px 'IBM Plex Mono', ui-monospace, monospace",
    grid: { stroke: cssColor("--faint"), width: 1 },
    ticks: { stroke: cssColor("--rule"), width: 1, size: 4 },
  };
}

// Midnight ticks show the date, the ones between show the hour.
function timeLabels(chart: uPlot, splits: number[]): string[] {
  return splits.map((split) => {
    const date = new Date(split * 1000);
    return date.getHours() === 0 ? shortDate.format(date) : hourOnly.format(date);
  });
}

function countRange(chart: uPlot, min: number, max: number | null): uPlot.Range.MinMax {
  return [0, uPlot.rangeNum(0, Math.max(max ?? 0, 1), 0.1, true)[1]];
}

const LEGEND_AT_END: uPlot.Hooks.Arrays = { ready: [legendAtEnd], setCursor: [legendAtEnd] };

function stagePaint(cutovers: number[]): uPlot.Hooks.Arrays {
  if (cutovers.length === 0) {
    return {};
  }
  return {
    drawClear: [
      (chart) => {
        paintStageBands(chart, cutovers);
      },
    ],
    draw: [
      (chart) => {
        paintStageLines(chart, cutovers);
      },
    ],
  };
}

function barPaths(options: uPlot.Series.BarsPathBuilderOpts): uPlot.Series.PathBuilder {
  if (uPlot.paths.bars === undefined) {
    throw new Error("this uPlot build draws no bars");
  }
  return uPlot.paths.bars(options);
}

function dayLabel(chart: uPlot, value: number | null): string {
  return value === null ? NO_VALUE : shortDate.format(value * 1000);
}

export function drawTotals(container: HTMLElement, days: number[], view: Series[], cutovers: number[]): uPlot {
  const colors = ["", ...view.map((series) => cssColor(series.color))];
  const data: uPlot.AlignedData = [days.map((day) => day / 1000)];
  for (const series of view) {
    const totals: number[] = [];
    let total = 0;
    for (const count of countPerDay(days, series.times, null)) {
      total += count;
      totals.push(total);
    }
    data.push(totals);
  }
  const options: uPlot.Options = {
    width: Math.max(320, container.clientWidth),
    height: 260,
    cursor: { y: false, sync: { key: "history" }, drag: { x: true, y: false } },
    legend: { markers: { width: 0, fill: (chart, i) => colors[i] } },
    scales: { x: { time: true }, y: { range: countRange } },
    axes: [
      { ...axis(), space: 60, incrs: DAY_STEPS, values: timeLabels },
      { ...axis(), size: 48 },
    ],
    series: [
      { label: "Day", value: dayLabel },
      ...view.map((series, i) => ({
        label: series.label,
        stroke: colors[i + 1],
        width: 2,
        fill: view.length === 1 ? `${colors[i + 1]}1f` : undefined,
        points: { show: false },
        value: (chart: uPlot, value: number | null) => (value === null ? NO_VALUE : number.format(value)),
      })),
    ],
    hooks: { ...LEGEND_AT_END, ...stagePaint(cutovers) },
  };
  return new uPlot(options, data, container);
}

// A series' bar reaches its own count plus every shown series after it. Taller bars draw first, so later ones cover their lower part.
function stackBars(counts: number[][], shown: boolean[]): number[][] {
  const stacked: number[][] = [];
  let below = counts[0].map(() => 0);
  for (let k = counts.length - 1; k >= 0; k--) {
    if (shown[k]) {
      below = below.map((total, i) => total + counts[k][i]);
    }
    stacked[k] = shown[k] ? below : counts[k];
  }
  return stacked;
}

function dailyData(x: number[], counts: number[][], shown: boolean[]): uPlot.AlignedData {
  const totals = x.map((value, i) => sum(counts.map((series, k) => (shown[k] ? series[i] : 0))));
  const average = totals.map((value, i) => mean(totals.slice(Math.max(0, i - 6), i + 1)));
  return [x, ...stackBars(counts, shown), average];
}

export function drawDaily(container: HTMLElement, days: number[], view: Series[], cutovers: number[], format: (value: number) => string): uPlot {
  const ink = cssColor("--ink");
  const colors = ["", ...view.map((series) => cssColor(series.color)), ink];
  const counts = view.map((series) => countPerDay(days, series.times, series.weights));
  const x = days.map((day) => day / 1000);
  const bars = barPaths({ size: [0.7, 48] });
  const options: uPlot.Options = {
    width: Math.max(320, container.clientWidth),
    height: 220,
    cursor: { y: false, sync: { key: "history" }, drag: { x: true, y: false }, points: { show: false } },
    legend: { markers: { width: 0, fill: (chart, i) => colors[i] } },
    scales: { x: { time: true, range: (chart, min, max) => [min - 43200, max + 43200] }, y: { range: countRange } },
    axes: [
      { ...axis(), space: 60, incrs: DAY_STEPS, values: timeLabels },
      { ...axis(), size: 48 },
    ],
    series: [
      { label: "Day", value: dayLabel },
      ...view.map((series, i) => ({
        label: series.label,
        fill: colors[i + 1],
        stroke: cssColor("--paper"),
        width: 1,
        paths: bars,
        points: { show: false },
        value: (chart: uPlot, value: number | null, seriesIdx: number, idx: number | null) => (idx === null ? NO_VALUE : format(counts[seriesIdx - 1][idx])),
      })),
      { label: "7-day average", stroke: ink, width: 2, points: { show: false }, value: (chart: uPlot, value: number | null) => (value === null ? NO_VALUE : oneDecimal.format(value)) },
    ],
    hooks: {
      ...LEGEND_AT_END,
      ...stagePaint(cutovers),
      setSeries: [
        (chart) => {
          chart.setData(
            dailyData(
              x,
              counts,
              chart.series.slice(1, -1).map((series) => series.show !== false),
            ),
          );
        },
      ],
    },
  };
  return new uPlot(
    options,
    dailyData(
      x,
      counts,
      view.map(() => true),
    ),
    container,
  );
}

export function drawMachine(container: HTMLElement, facts: History, spec: MachineChart, level: number | null): uPlot {
  const hours = facts.hours;
  const color = cssColor("--c1");
  const muted = cssColor("--muted");
  const colors = ["", color, muted];
  const data: uPlot.AlignedData = [hours.map((hour) => hour.hour), hours.map((hour) => hour[spec.key])];
  const series: uPlot.Series[] = [
    { label: "Hour", value: (chart, value: number | null) => (value === null ? NO_VALUE : dayTime.format(value * 1000)) },
    { label: spec.title, stroke: color, width: 2, points: { show: hours.length < 48, size: 5 }, value: (chart, value: number | null) => (value === null ? NO_VALUE : oneDecimal.format(value)) },
  ];
  if (level !== null) {
    series.push({ label: "Since start", stroke: muted, width: 1, dash: [4, 3], points: { show: false }, value: () => oneDecimal.format(level) });
    data.push(hours.map(() => level));
  }
  const options: uPlot.Options = {
    width: Math.max(320, container.clientWidth),
    height: 140,
    cursor: { y: false, sync: { key: "machine" }, drag: { x: true, y: false } },
    legend: { markers: { width: 0, fill: (chart, i) => colors[i] } },
    scales: { x: { time: true }, y: { range: countRange } },
    axes: [
      { ...axis(), space: 60, incrs: HOUR_STEPS, values: timeLabels },
      { ...axis(), size: 40 },
    ],
    series,
    hooks: { ...LEGEND_AT_END, ...stagePaint(facts.cutovers) },
  };
  return new uPlot(options, data, container);
}
