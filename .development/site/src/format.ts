export const REPO = "betterlite-ai/betterlite";
export const GITHUB = `https://github.com/${REPO}`;
export const DAY = 86400000;

export const number = new Intl.NumberFormat("en-US");
export const oneDecimal = new Intl.NumberFormat("en-US", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
export const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
export const shortDate = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" });
export const yearDate = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "short", day: "numeric" });
export const longDate = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
export const dayTime = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
export const hourOnly = new Intl.DateTimeFormat("en-US", { hour: "numeric" });

export function percent(part: number, whole: number): string {
  const share = whole === 0 ? 0 : (100 * part) / whole;
  return `${share.toFixed(1)}%`;
}

export function share(value: number): string {
  return `${(100 * value).toFixed(1)}%`;
}

export function gigabytes(value: number): string {
  return `${oneDecimal.format(value)} GB`;
}

export function idNumber(id: string): number {
  return Number(id.slice(1));
}

export function dayKey(time: number): string {
  const date = new Date(time);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function cssColor(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function duration(minutes: number): string {
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

export function sum(values: (number | null)[]): number {
  return values.reduce<number>((total, value) => total + (value ?? 0), 0);
}

export function mean(values: (number | null)[]): number | null {
  const known = values.filter((value) => value !== null);
  return known.length === 0 ? null : sum(known) / known.length;
}

// The list is sorted and has at least one value.
export function rank(sorted: number[], part: number): number {
  return sorted[Math.min(sorted.length - 1, Math.floor(part * sorted.length))];
}

export function quantile(values: (number | null)[], part: number): number | null {
  const sorted = values.filter((value) => value !== null).sort((a, b) => a - b);
  return sorted.length === 0 ? null : rank(sorted, part);
}

export function plural(count: number, word: string): string {
  return `${number.format(count)} ${word}${count === 1 ? "" : "s"}`;
}
