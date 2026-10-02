import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import { setDesktopLocale } from "./api/commands";
import { isDesktop } from "./api/platform";
import App from "./App";
import { locale } from "./i18n";
import { QUICK_ENTRY_WINDOW_LABEL, QuickEntry } from "./quickentry/QuickEntry";
import "./styles.css";

// The quick-entry popup (opened from the tray, `apps/desktop/src-tauri/src/tray.rs`) loads the
// same `index.html` as the main window — Tauri windows are otherwise separate top-level browser
// contexts, so there is no in-page routing to set up, just picking which React root to mount
// based on which window this particular webview belongs to. The web build has no such window.
const isQuickEntry = isDesktop && getCurrentWindow().label === QUICK_ENTRY_WINDOW_LABEL;

// The tray menu and notifications are drawn by the shell, which follows the main window's
// language (the quick-entry window isn't allowed this command, and shares the same choice).
if (isDesktop && !isQuickEntry) {
  setDesktopLocale(locale).catch(() => {
    // Not fatal: the shell keeps its default language.
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isQuickEntry ? <QuickEntry /> : <App />}</React.StrictMode>,
);
