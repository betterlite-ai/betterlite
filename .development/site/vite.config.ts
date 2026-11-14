import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// GitHub Pages serves the page under /betterlite/, so the built links stay relative. The bundle carries React, uPlot and the parsers, and licenses.md ships their notices with it.
export default defineConfig({ base: "./", plugins: [react()], build: { license: { fileName: "licenses.md" } } });
