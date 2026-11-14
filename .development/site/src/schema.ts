import { z } from "zod";

export const entrySchema = z.object({
  id: z.string(),
  milestone: z.string(),
  title: z.string(),
  status: z.string(),
  depends_on: z.array(z.string()),
  file: z.string().nullable(),
  covers: z.array(z.string()),
  parent: z.string().nullable(),
  resolution: z.string().nullable(),
  pr: z.number().nullable(),
});
export type Entry = z.infer<typeof entrySchema>;

export const deviationLogSchema = z.object({
  task: z.string(),
  deviations: z.array(z.object({ what: z.string(), why: z.string(), link: z.string() })),
});
export type DeviationLog = z.infer<typeof deviationLogSchema>;

export const configSchema = z.object({
  repository: z.string(),
  version: z.string(),
  commit: z.string(),
  excluded: z.array(z.object({ glob: z.string() })),
});
export type Config = z.infer<typeof configSchema>;

const text = z.string().transform((cell) => (cell === "" ? null : cell));
const count = z.string().min(1).pipe(z.coerce.number());
const optionalCount = z
  .string()
  .transform((cell) => (cell === "" ? null : Number(cell)))
  .pipe(z.number().nullable());

export const pullSchema = z.object({
  number: count,
  branch: z.string(),
  first: count,
  commits: count,
  opened: count,
  queued: optionalCount,
  queues: count,
  merged: count,
  lines: count,
  files: count,
});
export type Pull = z.infer<typeof pullSchema>;

export const runSchema = z.object({
  event: z.string(),
  conclusion: text,
  pr: optionalCount,
  branch: text,
  created: count,
  started: count,
  updated: count,
  attempt: count,
});
export type Run = z.infer<typeof runSchema>;

export const agentRunSchema = z.object({
  start: count,
  end: count,
  task: z.string(),
  role: z.string(),
  model: z.string(),
  calls: optionalCount,
  output_tokens: optionalCount,
  thinking_tokens: optionalCount,
  reply_s: optionalCount,
  first_s: optionalCount,
  compact_s: optionalCount,
  error_s: optionalCount,
  tool_s: optionalCount,
  test_s: optionalCount,
  build_s: optionalCount,
  lint_s: optionalCount,
  other_s: optionalCount,
  rest_s: optionalCount,
  short_n: optionalCount,
  short_s: optionalCount,
  mid_n: optionalCount,
  mid_s: optionalCount,
  long_n: optionalCount,
  long_s: optionalCount,
  xlong_n: optionalCount,
  xlong_s: optionalCount,
});
export type AgentRun = z.infer<typeof agentRunSchema>;

export const targetSchema = z.object({ gb: count, stage: count });
export type Target = z.infer<typeof targetSchema>;

export const bootSchema = z.object({
  from: count,
  to: count,
  read_tb: count,
  written_tb: count,
  cpu_busy: count,
  iowait: count,
  io_stall: count,
  memory_stall: count,
  swap_gb: count,
  memory_gb: count,
});
export type Boot = z.infer<typeof bootSchema>;

export const hourSchema = z.object({
  hour: count,
  minutes: count,
  written_gb_h: count,
  read_gb_h: count,
  cpu_busy: count,
  iowait: count,
  memory_used: count,
  swap_gb: count,
  disk_used: count,
  io_stall: count,
  flows: count,
  rustc: count,
});
export type Hour = z.infer<typeof hourSchema>;

export const exceptionSchema = z.object({
  kind: z.string(),
  where: z.string(),
  value: z.string(),
  default: z.string(),
  reason: z.string(),
  link: z.string(),
  date: z.string(),
});
export type Exception = z.infer<typeof exceptionSchema>;

export const openPullSchema = z.object({
  number: z.number(),
  title: z.string(),
  html_url: z.string(),
  created_at: z.string(),
  head: z.object({ ref: z.string() }),
});
export type OpenPull = z.infer<typeof openPullSchema>;
