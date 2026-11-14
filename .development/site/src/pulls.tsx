import type { Merge } from "./backlog.ts";
import { shortDate } from "./format.ts";
import { ShortTable, TaskLink } from "./parts.tsx";
import type { Entry } from "./schema.ts";
import { taskOfPull, type Review } from "./site.ts";

export function OpenPulls({ review, byId }: { review: Review; byId: Map<string, Entry> }) {
  if (review.state === "asking") {
    return (
      <div className="table-scroll" id="open-prs">
        <p className="empty">Asking GitHub</p>
      </div>
    );
  }
  if (review.state === "failed") {
    return (
      <div className="table-scroll" id="open-prs">
        <p className="empty">GitHub's API didn't answer ({review.message}). It allows 60 requests an hour per address without a login.</p>
      </div>
    );
  }
  if (review.pulls.length === 0) {
    return (
      <div className="table-scroll" id="open-prs">
        <p className="empty">No pull request is open right now.</p>
      </div>
    );
  }
  const head = (
    <thead>
      <tr>
        <th>PR</th>
        <th>Title</th>
        <th className="num">Open for</th>
      </tr>
    </thead>
  );
  const rows = review.pulls.map(({ pull, age }) => {
    const entry = taskOfPull(pull, byId);
    return (
      <tr key={pull.number}>
        <td className="id">
          <a href={pull.html_url}>#{pull.number}</a>
        </td>
        <td>
          {pull.title}
          {entry !== null && (
            <>
              {" "}
              <span className="muted">{entry.milestone}</span>
            </>
          )}
        </td>
        <td className="num muted">{age}</td>
      </tr>
    );
  });
  // GitHub lists the newest pull request first.
  return (
    <div className="table-scroll" id="open-prs">
      <ShortTable head={head} rows={rows} rest="older pull requests" shown={3} />
    </div>
  );
}

export function LatestMerges({ merges }: { merges: Merge[] }) {
  if (merges.length === 0) {
    return (
      <div className="table-scroll" id="merges">
        <p className="empty">No task has merged yet.</p>
      </div>
    );
  }
  const head = (
    <thead>
      <tr>
        <th>Date</th>
        <th>Task</th>
        <th>Title</th>
        <th>Milestone</th>
      </tr>
    </thead>
  );
  const rows = merges.map((merge) => (
    <tr key={merge.entry.id}>
      <td className="muted date">{shortDate.format(merge.time)}</td>
      <td>
        <TaskLink entry={merge.entry} />
      </td>
      <td>{merge.entry.title}</td>
      <td className="muted">{merge.entry.milestone}</td>
    </tr>
  ));
  return (
    <div className="table-scroll" id="merges">
      <ShortTable head={head} rows={rows} rest="older merges" />
    </div>
  );
}
