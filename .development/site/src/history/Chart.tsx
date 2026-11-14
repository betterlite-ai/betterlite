import { useEffect, useRef, useSyncExternalStore } from "react";
import type uPlot from "uplot";

const DARK = "(prefers-color-scheme: dark)";

function watchScheme(notify: () => void): () => void {
  const query = matchMedia(DARK);
  query.addEventListener("change", notify);
  return () => {
    query.removeEventListener("change", notify);
  };
}

export function Chart({ id, draw, apply }: { id: string; draw: (container: HTMLElement) => uPlot; apply?: (chart: uPlot) => void }) {
  const holder = useRef<HTMLDivElement>(null);
  const drawn = useRef<uPlot | null>(null);
  // A chart reads its colors when it is drawn, so a theme change draws it again.
  const dark = useSyncExternalStore(watchScheme, () => matchMedia(DARK).matches);
  useEffect(() => {
    const container = holder.current;
    if (container === null) {
      return;
    }
    const chart = draw(container);
    drawn.current = chart;
    // A hidden tab has no width, so a chart takes its width when its tab shows, and again on every resize.
    const resizer = new ResizeObserver((records) => {
      const width = Math.floor(records[0].contentRect.width);
      if (width > 0) {
        chart.setSize({ width, height: chart.height });
      }
    });
    resizer.observe(container);
    return () => {
      drawn.current = null;
      resizer.disconnect();
      chart.destroy();
    };
  }, [draw, dark]);
  // Runs after every draw as well, so a chart drawn again keeps what the reader switched off, and a change alone keeps the zoom.
  useEffect(() => {
    if (drawn.current !== null && apply !== undefined) {
      apply(drawn.current);
    }
  }, [apply, draw, dark]);
  return <div className="chart" id={id} ref={holder} />;
}
