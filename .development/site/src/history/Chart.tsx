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

export function Chart({ id, draw }: { id: string; draw: (container: HTMLElement) => uPlot }) {
  const holder = useRef<HTMLDivElement>(null);
  // A chart reads its colors when it is drawn, so a theme change draws it again.
  const dark = useSyncExternalStore(watchScheme, () => matchMedia(DARK).matches);
  useEffect(() => {
    const container = holder.current;
    if (container === null) {
      return;
    }
    const chart = draw(container);
    // A hidden tab has no width, so a chart takes its width when its tab shows, and again on every resize.
    const resizer = new ResizeObserver((records) => {
      const width = Math.floor(records[0].contentRect.width);
      if (width > 0) {
        chart.setSize({ width, height: chart.height });
      }
    });
    resizer.observe(container);
    return () => {
      resizer.disconnect();
      chart.destroy();
    };
  }, [draw, dark]);
  return <div className="chart" id={id} ref={holder} />;
}
