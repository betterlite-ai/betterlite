import { number } from "../format.ts";
import { ShortTable } from "../parts.tsx";
import type { Exception } from "../schema.ts";

const GROUPS = [
  {
    kind: "expects",
    title: "Lint escapes",
    numbers: true,
    rest: "more tasks",
    about: "New #[expect(...)] attributes and Python lines with a cast, type: ignore, pyright: ignore or noqa that a merged task was allowed. The default is none.",
  },
  { kind: "lines", title: "Raised line budgets", numbers: true, rest: "more tasks", about: "Merged tasks whose changed-line budget went up after planning, against the planned budget." },
  { kind: "advisory", title: "Ignored advisories", numbers: false, rest: "more advisories", about: "RustSec advisories cargo deny lets through. The reason is the one in deny.toml, or the commit that added the entry." },
  { kind: "duplicate", title: "Duplicate crate versions", numbers: false, rest: "more crates", about: "Older crate versions cargo deny allows next to a newer one, with the commit that added each." },
];

export function CheckLater({ exceptions, hidden }: { exceptions: Exception[]; hidden: boolean }) {
  return (
    <section id="check-later" hidden={hidden}>
      <div className="section-head">
        <h2>Check later</h2>
        <p>
          Exceptions allowed on purpose that are worth a second look: lint escapes and raised line budgets in merged tasks, and what cargo deny lets through. The gate lists them from the task files, the budget commits and deny.toml at every
          deploy, newest first. Deviations from LiteLLM have their own <a href="#deviations">tab</a>.
        </p>
      </div>
      <div id="exceptions">
        {GROUPS.map((group) => {
          const listed = exceptions.filter((row) => row.kind === group.kind);
          const head = (
            <thead>
              <tr>
                <th>Where</th>
                {group.numbers && <th className="num">Allowed</th>}
                {group.numbers && <th className="num">Default</th>}
                <th>Reason</th>
                <th>Date</th>
              </tr>
            </thead>
          );
          const rows = listed.map((row, i) => (
            <tr key={i}>
              <td className="id">
                <a href={row.link}>{row.where}</a>
              </td>
              {group.numbers && <td className="num">{row.value}</td>}
              {group.numbers && <td className="num muted">{row.default}</td>}
              <td>{row.reason}</td>
              <td className="muted date">{row.date}</td>
            </tr>
          ));
          return (
            <div className="block" key={group.kind}>
              <header>
                <h3>
                  {group.title}: {number.format(listed.length)}
                </h3>
                <p>{group.about}</p>
              </header>
              {listed.length === 0 ? <p className="empty">Nothing to check.</p> : <ShortTable head={head} rows={rows} rest={group.rest} />}
            </div>
          );
        })}
      </div>
    </section>
  );
}
