import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { QUICK_ENTRY_WINDOW_LABEL, QuickEntry } from "./quickentry/QuickEntry";
import "./styles.css";

// The quick-entry popup (opened from the tray, `apps/desktop/src-tauri/src/tray.rs`) loads the
// same `index.html` as the main window — Tauri windows are otherwise separate top-level browser
// contexts, so there is no in-page routing to set up, just picking which React root to mount
// based on which window this particular webview belongs to.
const isQuickEntry = getCurrentWindow().label === QUICK_ENTRY_WINDOW_LABEL;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isQuickEntry ? <QuickEntry /> : <App />}</React.StrictMode>,
);
