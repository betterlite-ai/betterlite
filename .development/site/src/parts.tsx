import { useState, type ReactNode } from "react";
import { GITHUB, number, percent } from "./format.ts";

export function TaskLink({ entry }: { entry: { id: string; pr: number | null } }) {
  const href = entry.pr === null ? `${GITHUB}/blob/main/.development/backlog/${entry.id}.yaml` : `${GITHUB}/pull/${entry.pr}`;
  return (
    <a className="id" href={href}>
      {entry.id}
    </a>
  );
}

export function ProgressBar({ part, whole }: { part: number; whole: number }) {
  const width = whole === 0 ? 0 : (100 * part) / whole;
  return (
    <div className="bar" role="img" aria-label={`${percent(part, whole)} done`}>
      <div className="track">
        <div className="fill" style={{ width: `${width.toFixed(2)}%` }} />
      </div>
      <span className="pct">{percent(part, whole)}</span>
    </div>
  );
}

// The first rows, five unless the caller says otherwise. The rest join the same table on a click, so every row shares its columns.
export function ShortTable({ head, rows, rest, shown = 5 }: { head: ReactNode; rows: ReactNode[]; rest: string; shown?: number }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div className="table-scroll">
        <table>
          {head}
          <tbody>{open ? rows : rows.slice(0, shown)}</tbody>
        </table>
      </div>
      {rows.length > shown && (
        <details
          onToggle={(event) => {
            setOpen(event.currentTarget.open);
          }}
        >
          <summary>{open ? "Show fewer" : `Show ${number.format(rows.length - shown)} ${rest}`}</summary>
        </details>
      )}
    </div>
  );
}

export function Figure({ value, small, label }: { value: ReactNode; small: ReactNode; label: string }) {
  return (
    <div className="figure">
      <div className="value">
        {value} <small>{small}</small>
      </div>
      <div className="label">{label}</div>
    </div>
  );
}

export function Swatch({ color }: { color: string }) {
  return <i className="swatch" style={{ background: `var(${color})` }} aria-hidden="true" />;
}

export interface Mode<T extends string> {
  value: T;
  label: string;
}

export function Modes<T extends string>({ label, modes, value, onChange }: { label: string; modes: Mode<T>[]; value: T; onChange: (value: T) => void }) {
  return (
    <div className="modes" role="group" aria-label={label}>
      {modes.map((mode) => (
        <button
          key={mode.value}
          type="button"
          aria-pressed={mode.value === value}
          onClick={() => {
            onChange(mode.value);
          }}
        >
          {mode.label}
        </button>
      ))}
    </div>
  );
}
