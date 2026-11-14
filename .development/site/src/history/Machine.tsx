import { useCallback } from "react";
import { dayTime, oneDecimal, plural, yearDate } from "../format.ts";
import type { Boot } from "../schema.ts";
import { Chart } from "./Chart.tsx";
import { drawMachine, type MachineChart } from "./charts.ts";
import { weighted, type History, type HourKey } from "./facts.ts";
import { StageLabel } from "./tables.tsx";

interface MachineRow {
  label: string;
  unit: string;
  key: HourKey | null;
  boot: ((boot: Boot) => number) | null;
}

function bootHours(boot: Boot): number {
  return (boot.to - boot.from) / 3600;
}

const MACHINE_ROWS: MachineRow[] = [
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

const MACHINE_CHARTS: MachineChart[] = [
  { title: "SSD written", unit: "GB an hour", key: "written_gb_h" },
  { title: "CPU busy", unit: "%", key: "cpu_busy" },
  { title: "Disk stall", unit: "% of time", key: "io_stall" },
  { title: "RAM in use", unit: "%", key: "memory_used" },
  { title: "rustc processes", unit: "average", key: "rustc" },
  { title: "Agent flows running", unit: "average", key: "flows" },
];

function MachineValue({ value, unit }: { value: number | null; unit: string }) {
  if (value === null) {
    return <span className="muted">not sampled</span>;
  }
  return `${oneDecimal.format(value)}${unit === "%" ? "%" : ""}`;
}

export function MachineNote({ history }: { history: History }) {
  const last = history.hours.at(-1);
  return (
    <p id="machine-note">
      {history.boot !== null && `Since start covers ${yearDate.format(history.boot.from * 1000)} to ${yearDate.format(history.boot.to * 1000)}, read from the laptop's own counters. `}
      {last === undefined ? "No 5-minute sample has been taken yet." : `The 5-minute samples cover ${plural(history.hours.length, "hour")} up to ${dayTime.format((last.hour + 3600) * 1000)}.`} The SSD numbers count every write on the
      laptop, not only the agents'.
    </p>
  );
}

export function MachineTable({ history }: { history: History }) {
  const hours = history.hours;
  const boot = history.boot;
  const last = hours.findLast((hour) => hour.minutes >= 45) ?? hours.at(-1) ?? null;
  return (
    <div className="table-scroll" id="machine-table">
      <table>
        <thead>
          <tr>
            <th>Measure</th>
            <th>Unit</th>
            <th className="num">Since start</th>
            {history.live.map((period) => (
              <th key={period.key} className="num">
                <StageLabel period={period} /> samples
              </th>
            ))}
            <th className="num">Last full hour</th>
          </tr>
        </thead>
        <tbody>
          {MACHINE_ROWS.map((row) => (
            <tr key={row.label}>
              <td>{row.label}</td>
              <td className="muted">{row.unit === "%" ? "" : row.unit}</td>
              <td className="num">
                <MachineValue value={boot === null || row.boot === null ? null : row.boot(boot)} unit={row.unit} />
              </td>
              {history.live.map((period) => {
                const inside = hours.filter((hour) => hour.stage === period.key);
                return (
                  <td key={period.key} className="num">
                    <MachineValue value={row.key === null || inside.length === 0 ? null : weighted(inside, row.key)} unit={row.unit} />
                  </td>
                );
              })}
              <td className="num">
                <MachineValue value={row.key === null || last === null ? null : last[row.key]} unit={row.unit} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function MachinePlot({ history, spec }: { history: History; spec: MachineChart }) {
  const reference = MACHINE_ROWS.find((row) => row.key === spec.key)?.boot ?? null;
  const level = history.boot === null || reference === null ? null : reference(history.boot);
  const draw = useCallback((container: HTMLElement) => drawMachine(container, history, spec, level), [history, spec, level]);
  return (
    <div>
      <h4>{spec.title}</h4>
      <p>{spec.unit}</p>
      {history.hours.length === 0 ? (
        <div className="chart">
          <p className="empty">Not sampled yet.</p>
        </div>
      ) : (
        <Chart id={`machine-${spec.key}`} draw={draw} />
      )}
    </div>
  );
}

export function MachineCharts({ history }: { history: History }) {
  return (
    <div className="machine" id="machine">
      {MACHINE_CHARTS.map((spec) => (
        <MachinePlot key={spec.key} history={history} spec={spec} />
      ))}
    </div>
  );
}
