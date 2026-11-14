import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "uplot/dist/uPlot.min.css";
import { App } from "./App.tsx";
import { loadOpenPulls, loadSite } from "./site.ts";
import "./style.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("index.html has no #root element");
}
createRoot(root).render(
  <StrictMode>
    <App loading={loadSite()} asking={loadOpenPulls()} />
  </StrictMode>,
);
