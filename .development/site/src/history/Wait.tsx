import { number, oneDecimal, plural, share, sum, yearDate } from "../format.ts";
import { Swatch } from "../parts.tsx";
import { totals, WAIT_KINDS, type History, type Measured, type Period, type Tally } from "./facts.ts";
import { Cell, StageLabel } from "./tables.tsx";

const REPLY_SIZES: { label: string; count: Tally; seconds: Tally }[] = [
  { label: "Under 500", count: "short_n", seconds: "short_s" },
  { label: "500 to 1,999", count: "mid_n", seconds: "mid_s" },
  { label: "2,000 to 4,999", count: "long_n", seconds: "long_s" },
  { label: "5,000 and up", count: "xlong_n", seconds: "xlong_s" },
];

function WaitSplit({ history, period }: { history: History; period: Measured }) {
  const workers = history.traced.filter((run) => run.stage === period.key);
  const seconds = WAIT_KINDS.map((kind) => totals(workers, kind.keys));
  const total = sum(seconds);
  if (total === 0) {
    return (
      <>
        <h4>
          <StageLabel period={period} />
        </h4>
        <p className="empty">No Claude worker transcript yet.</p>
      </>
    );
  }
  return (
    <>
      <h4>
        <StageLabel period={period} />, {plural(workers.length, "worker run")}, {number.format(Math.round(total / 3600))} hours
      </h4>
      <div className="split" aria-hidden="true">
        {WAIT_KINDS.map((kind, i) => seconds[i] > 0 && <span key={kind.label} style={{ flexGrow: seconds[i], background: `var(${kind.color})` }} title={`${kind.label}: ${share(seconds[i] / total)}`} />)}
      </div>
      <div className="keys">
        {WAIT_KINDS.map((kind, i) => (
          <span key={kind.label}>
            <Swatch color={kind.color} />
            {kind.label} <b>{share(seconds[i] / total)}</b> {number.format(Math.round(seconds[i] / 3600))} h
          </span>
        ))}
      </div>
    </>
  );
}

function WaitNote({ history }: { history: History }) {
  const traced = history.traced;
  const replies = totals(traced, ["reply_s"]);
  if (replies === 0) {
    return <p className="table-note" id="wait-note" />;
  }
  const calls = totals(traced, ["calls"]);
  const tokens = totals(traced, ["output_tokens"]);
  const first = yearDate.format(Math.min(...traced.map((run) => run.start)) * 1000);
  return (
    <p className="table-note" id="wait-note">
      The transcripts start on {first}, so older runs and Codex runs are not here. Gaps over 15 minutes and the time between a worker's attempts are left out. {share(totals(traced, ["first_s"]) / replies)} of the reply time passes before
      the first part of a reply shows up. The model thinks in that time, and {share(totals(traced, ["thinking_tokens"]) / tokens)} of all output tokens are thinking. Replies of 2,000 output tokens or more are{" "}
      {share(totals(traced, ["long_n", "xlong_n"]) / calls)} of the replies and take {share(totals(traced, ["long_s", "xlong_s"]) / replies)} of the reply time.
    </p>
  );
}

// One row that names a stage, ahead of that stage's rows.
function StageRow({ period, empty }: { period: Period; empty: boolean }) {
  return (
    <tr>
      <th scope="rowgroup" colSpan={5}>
        <StageLabel period={period} />
        {empty && <span className="muted">, no Claude worker transcript yet</span>}
      </th>
    </tr>
  );
}

function ReplySizes({ history }: { history: History }) {
  return (
    <div className="table-scroll" id="reply-sizes">
      <table>
        <thead>
          <tr>
            <th>Output tokens in a reply</th>
            <th className="num">Replies</th>
            <th className="num">Hours</th>
            <th className="num">Of reply time</th>
            <th className="num">Mean wait</th>
          </tr>
        </thead>
        {history.live.map((period) => {
          const workers = history.traced.filter((run) => run.stage === period.key);
          const all = totals(workers, ["reply_s"]);
          return (
            <tbody key={period.key}>
              <StageRow period={period} empty={workers.length === 0} />
              {workers.length > 0 &&
                REPLY_SIZES.map((size) => {
                  const count = totals(workers, [size.count]);
                  const seconds = totals(workers, [size.seconds]);
                  return (
                    <tr key={size.label}>
                      <td>{size.label}</td>
                      <td className="num">{number.format(count)}</td>
                      <td className="num">{number.format(Math.round(seconds / 3600))}</td>
                      <td className="num">
                        <Cell value={all === 0 ? null : seconds / all} format={share} missing="none" />
                      </td>
                      <td className="num">
                        <Cell value={count === 0 ? null : seconds / count} format={(value) => `${oneDecimal.format(value)} s`} missing="none" />
                      </td>
                    </tr>
                  );
                })}
            </tbody>
          );
        })}
      </table>
    </div>
  );
}

function Speeds({ history }: { history: History }) {
  return (
    <div className="table-scroll" id="speeds">
      <table>
        <thead>
          <tr>
            <th>Worker model</th>
            <th className="num">Replies</th>
            <th className="num">Tokens each</th>
            <th className="num">Tokens a second</th>
            <th className="num">Thinking</th>
          </tr>
        </thead>
        {history.live.map((period) => {
          const workers = history.traced.filter((run) => run.stage === period.key);
          const calls = new Map<string, number>();
          for (const run of workers) {
            calls.set(run.model, (calls.get(run.model) ?? 0) + (run.calls ?? 0));
          }
          const models = [...calls.entries()].filter(([, count]) => count > 0).sort((a, b) => b[1] - a[1]);
          return (
            <tbody key={period.key}>
              <StageRow period={period} empty={models.length === 0} />
              {models.map(([model, count]) => {
                const runs = workers.filter((run) => run.model === model);
                const tokens = totals(runs, ["output_tokens"]);
                const seconds = totals(runs, ["reply_s"]);
                return (
                  <tr key={model}>
                    <td>{model}</td>
                    <td className="num">{number.format(count)}</td>
                    <td className="num">{number.format(Math.round(tokens / count))}</td>
                    <td className="num">
                      <Cell value={seconds === 0 ? null : tokens / seconds} format={(value) => number.format(Math.round(value))} missing="none" />
                    </td>
                    <td className="num">
                      <Cell value={tokens === 0 ? null : totals(runs, ["thinking_tokens"]) / tokens} format={share} missing="none" />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          );
        })}
      </table>
      <p className="table-note">Tokens a second divides a model's output tokens by the time spent waiting for its replies, so the wait before the first token counts too.</p>
    </div>
  );
}

export function Wait({ history }: { history: History }) {
  return (
    <div>
      <div id="wait">
        {history.live.map((period) => (
          <WaitSplit key={period.key} history={history} period={period} />
        ))}
      </div>
      <WaitNote history={history} />
      <h4>Reply size</h4>
      <ReplySizes history={history} />
      <h4>Model speed</h4>
      <Speeds history={history} />
    </div>
  );
}
