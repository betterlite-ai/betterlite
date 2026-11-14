import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { GITHUB, longDate, yearDate } from "./format.ts";
import { History } from "./history/History.tsx";
import type { OpenPull } from "./schema.ts";
import { taskOfPull, type Review, type Site, type WaitingPull } from "./site.ts";
import { CheckLater } from "./tabs/CheckLater.tsx";
import { Deviations } from "./tabs/Deviations.tsx";
import { Milestones } from "./tabs/Milestones.tsx";
import { Lede, Overview } from "./tabs/Overview.tsx";

const TABS = [
  { id: "overview", label: "Overview" },
  { id: "history", label: "History" },
  { id: "milestones", label: "Milestones" },
  { id: "deviations", label: "Deviations" },
  { id: "check-later", label: "Check later" },
];

function watchHash(notify: () => void): () => void {
  window.addEventListener("hashchange", notify);
  return () => {
    window.removeEventListener("hashchange", notify);
  };
}

function Built({ site }: { site: Site }) {
  return (
    <span className="built" id="built" title={`Committed ${longDate.format(site.committed)}`}>
      built from <a href={`${GITHUB}/commit/${site.sha}`}>{site.sha.slice(0, 8)}</a>, {yearDate.format(site.committed)}
    </span>
  );
}

export function App({ loading, asking }: { loading: Promise<Site>; asking: Promise<WaitingPull[]> }) {
  const [site, setSite] = useState<Site | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [review, setReview] = useState<Review>({ state: "asking" });
  const hash = useSyncExternalStore(watchHash, () => location.hash);
  // #milestone-M5 opens the Milestones tab at that milestone.
  const wanted = hash.startsWith("#milestone-") ? "milestones" : hash.slice(1);
  const current = TABS.find((tab) => tab.id === wanted) ?? TABS[0];

  useEffect(() => {
    loading.then(setSite, (error: unknown) => {
      setFailure(error instanceof Error ? error.message : String(error));
    });
  }, [loading]);

  useEffect(() => {
    asking.then(
      (pulls) => {
        setReview({ state: "answered", pulls });
      },
      (error: unknown) => {
        setReview({ state: "failed", message: error instanceof Error ? error.message : String(error) });
      },
    );
  }, [asking]);

  const reviewing = useMemo(() => {
    if (site === null || review.state === "asking") {
      return null;
    }
    const tasks = new Map<string, OpenPull>();
    for (const { pull } of review.state === "answered" ? review.pulls : []) {
      const entry = taskOfPull(pull, site.byId);
      if (entry !== null) {
        tasks.set(entry.id, pull);
      }
    }
    return tasks;
  }, [site, review]);

  useEffect(() => {
    if (!hash.startsWith("#milestone-")) {
      window.scrollTo(0, 0);
    }
  }, [hash]);

  // The milestone list waits for GitHub's answer, so a link to a milestone opens it once the list is there.
  const listed = reviewing !== null;
  useEffect(() => {
    const milestone = listed && hash.startsWith("#milestone-") ? document.getElementById(hash.slice(1)) : null;
    if (milestone instanceof HTMLDetailsElement) {
      milestone.open = true;
      milestone.scrollIntoView();
    }
  }, [hash, listed]);

  return (
    <>
      <header className="masthead">
        <div className="wrap">
          <div className="masthead-id">
            <a className="brand" href="#overview">
              betterlite <span>progress</span>
            </a>
            {site !== null && <Built site={site} />}
          </div>
          <nav aria-label="Sections">
            {TABS.map((tab) => (
              <a key={tab.id} href={`#${tab.id}`} aria-current={tab === current}>
                {tab.label}
              </a>
            ))}
          </nav>
          <a className="repo" href={GITHUB} aria-label="betterlite on GitHub" title="betterlite on GitHub">
            <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
              <path d="M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 .468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 1-.5c.266-.265.47-.5.657-.656" />
            </svg>
          </a>
        </div>
      </header>

      <main className="wrap">
        {site === null ? (
          <section id="overview">
            <Lede config={null} status={failure === null ? "Loading the task list" : `The page couldn't load its data: ${failure}`} failed={failure !== null} site={null} />
          </section>
        ) : (
          <>
            <Overview site={site} hidden={current.id !== "overview"} />
            <History site={site} review={review} hidden={current.id !== "history"} />
            <Milestones site={site} reviewing={reviewing} answered={review.state === "answered"} hidden={current.id !== "milestones"} />
            <Deviations site={site} hidden={current.id !== "deviations"} />
            <CheckLater exceptions={site.exceptions} hidden={current.id !== "check-later"} />
          </>
        )}
      </main>

      <footer>
        <div className="wrap">
          <p>
            Data: the task list in <a href={`${GITHUB}/tree/main/.development/backlog`}>.development/backlog</a> and the logs in <a href={`${GITHUB}/tree/main/docs/deviations`}>docs/deviations</a>. Pull requests and CI runs are read from
            GitHub at every deploy. Agent runs and the build machine come from the snapshot in <a href={`${GITHUB}/tree/main/.development/site/stats`}>.development/site/stats</a>. Source of this page:{" "}
            <a href={`${GITHUB}/tree/main/.development/site`}>.development/site</a>.
          </p>
        </div>
      </footer>
    </>
  );
}
