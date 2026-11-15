import { milestoneOrder, PATHS } from "../backlog.ts";
import { number, percent, sum } from "../format.ts";
import { Figure, ProgressBar } from "../parts.tsx";
import type { Config } from "../schema.ts";
import type { Site } from "../site.ts";

export function Lede({ config, status, failed, site }: { config: Config | null; status: string | null; failed: boolean; site: Site | null }) {
  return (
    <div className="lede">
      <h1>Porting LiteLLM to Rust</h1>
      <p>
        betterlite rewrites{" "}
        {config === null ? (
          <a id="litellm-link" href="https://github.com/BerriAI/litellm">
            LiteLLM
          </a>
        ) : (
          <a id="litellm-link" href={`${config.repository}/tree/${config.commit}`}>
            LiteLLM {config.version}
          </a>
        )}{" "}
        in Rust under the MIT license. Coding agents build it one task at a time, and a task merges once its gate passes. This page reads the task list on main and is rebuilt after every merge.
      </p>
      {status !== null && (
        <p className={failed ? "status error" : "status"} id="status" role="status">
          {status}
        </p>
      )}
      {site !== null && <Figures site={site} />}
    </div>
  );
}

function Figures({ site }: { site: Site }) {
  const files = sum(site.areas.map((area) => area.files));
  const ported = sum(site.areas.map((area) => area.ported));
  return (
    <div className="figures" id="figures">
      <Figure value={number.format(site.done)} small={`of ${number.format(site.counted.length)}`} label="Porting tasks done" />
      <Figure value={percent(ported, files)} small={`of ${number.format(files)} files`} label="LiteLLM's Python ported" />
      <Figure value={number.format(site.lastWeek)} small={`${(site.lastWeek / 7).toFixed(1)} a day`} label="Merged in the last 7 days" />
      <Figure value={number.format(site.paths[0].chain.length)} small="tasks" label={`Longest open chain: ${PATHS[0].name}`} />
    </div>
  );
}

function MilestoneTable({ site }: { site: Site }) {
  return (
    <div className="table-scroll" id="milestone-table">
      <table>
        <thead>
          <tr>
            <th>Milestone</th>
            <th className="num">Done</th>
            <th className="num">Tasks</th>
            <th>Progress</th>
          </tr>
        </thead>
        <tbody>
          {[...site.milestones.values()].sort(milestoneOrder).map((group) => (
            <tr key={group.code}>
              <td>
                <a href={`#milestone-${group.code}`}>{group.name}</a>
              </td>
              <td className="num">{number.format(group.done)}</td>
              <td className="num">{number.format(group.total)}</td>
              <td>
                <ProgressBar part={group.done} whole={group.total} />
              </td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <td>All porting milestones</td>
            <td className="num">{number.format(site.done)}</td>
            <td className="num">{number.format(site.counted.length)}</td>
            <td>
              <ProgressBar part={site.done} whole={site.counted.length} />
            </td>
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

function Areas({ site }: { site: Site }) {
  const files = sum(site.areas.map((area) => area.files));
  return (
    <div className="table-scroll" id="areas">
      <table>
        <thead>
          <tr>
            <th>Area</th>
            <th className="num">Files</th>
            <th className="num">Fully ported</th>
            <th>Ported share</th>
          </tr>
        </thead>
        <tbody>
          {site.areas.map((area) => (
            <tr key={area.name}>
              <td>{area.name}</td>
              <td className="num">{number.format(area.files)}</td>
              <td className="num">{number.format(area.full)}</td>
              <td>
                <ProgressBar part={area.ported} whole={area.files} />
              </td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <td>All</td>
            <td className="num">{number.format(files)}</td>
            <td className="num">{number.format(sum(site.areas.map((area) => area.full)))}</td>
            <td>
              <ProgressBar part={sum(site.areas.map((area) => area.ported))} whole={files} />
            </td>
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

export function Overview({ site, hidden }: { site: Site; hidden: boolean }) {
  return (
    <section id="overview" hidden={hidden}>
      <Lede config={site.config} status={null} failed={false} site={site} />
      <div className="block">
        <header>
          <h3>Milestones</h3>
          <p>
            Tasks merged in each milestone. A milestone groups related work, not a build order: many tasks depend on work in later milestones. R milestones hold cleanup and refactoring of code already ported, so the porting counts leave
            them out.
          </p>
        </header>
        <MilestoneTable site={site} />
      </div>
      <div className="block">
        <header>
          <h3>Areas of LiteLLM</h3>
          <p>LiteLLM's Python files by folder. A file counts as ported in the share its merged tasks make up of all tasks that cover it.</p>
        </header>
        <Areas site={site} />
      </div>
    </section>
  );
}
