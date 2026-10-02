/**
 * Which shell this UI runs in. The same build serves the Tauri desktop app and the web build
 * (`crates/server`, `docs/HACKATHON.md` §1). Tauri injects `__TAURI_INTERNALS__` into its
 * webviews before any script runs; a browser never has it.
 */
export const isDesktop: boolean = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** The web build: accounts and server-configured models, no OS integration. */
export const isWeb: boolean = !isDesktop;
