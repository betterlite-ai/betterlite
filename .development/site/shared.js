export const REPO = "betterlite-ai/betterlite";
export const GITHUB = `https://github.com/${REPO}`;
export const DAY = 86400000;

const ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;" };
export const number = new Intl.NumberFormat("en-US");
export const oneDecimal = new Intl.NumberFormat("en-US", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
export const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
export const shortDate = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" });
export const yearDate = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "short", day: "numeric" });
export const longDate = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
export const dayTime = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
export const hourOnly = new Intl.DateTimeFormat("en-US", { hour: "numeric" });

export function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (c) => ESCAPES[c]);
}

export function percent(part, whole) {
  const share = whole === 0 ? 0 : (100 * part) / whole;
  return `${share.toFixed(1)}%`;
}

export function idNumber(id) {
  return Number(id.slice(1));
}

export function figure(value, small, label) {
  return `<div class="figure"><div class="value">${value} <small>${small}</small></div><div class="label">${label}</div></div>`;
}

export function dayKey(time) {
  const date = new Date(time);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function cssColor(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function duration(minutes) {
  if (minutes < 1) {
    return "under 1 min";
  }
  const total = Math.round(minutes);
  if (total < 60) {
    return `${total} min`;
  }
  if (total < 2880) {
    return total % 60 === 0 ? `${total / 60} h` : `${Math.floor(total / 60)} h ${total % 60} min`;
  }
  const hours = Math.round(minutes / 60);
  return hours % 24 === 0 ? `${hours / 24} d` : `${Math.floor(hours / 24)} d ${hours % 24} h`;
}

export function quantile(values, share) {
  const sorted = values.filter((value) => value !== null).sort((a, b) => a - b);
  return sorted.length === 0 ? null : sorted[Math.min(sorted.length - 1, Math.floor(share * sorted.length))];
}

export function plural(count, word) {
  return `${number.format(count)} ${word}${count === 1 ? "" : "s"}`;
}

export async function fetchText(path) {
  const response = await fetch(path, { cache: "no-cache" });
  if (!response.ok) {
    throw new Error(`${path} answered HTTP ${response.status}`);
  }
  return response.text();
}

// jq's @csv quotes every string and leaves numbers bare, so a quoted cell stays text and an empty bare cell is null.
function csvValue(cell, quoted) {
  if (quoted) {
    return cell;
  }
  if (cell === "") {
    return null;
  }
  const value = Number(cell);
  return Number.isNaN(value) ? cell : value;
}

function csvCells(line) {
  const cells = [];
  let cell = "";
  let inside = false;
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (inside && c === "\"" && line[i + 1] === "\"") {
      cell += "\"";
      i += 1;
    } else if (c === "\"") {
      inside = !inside;
      quoted = true;
    } else if (c === "," && !inside) {
      cells.push(csvValue(cell, quoted));
      cell = "";
      quoted = false;
    } else {
      cell += c;
    }
  }
  cells.push(csvValue(cell, quoted));
  return cells;
}

export function parseCsv(text) {
  const [head, ...lines] = text.trim().split("\n");
  const names = head.split(",");
  return lines.filter((line) => line !== "").map((line) => {
    const cells = csvCells(line);
    return Object.fromEntries(names.map((name, i) => [name, cells[i] ?? null]));
  });
}
