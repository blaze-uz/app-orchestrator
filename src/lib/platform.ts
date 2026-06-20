// Lightweight, dependency-free platform detection for UI affordances. Runs in
// the Tauri webview (WKWebView on macOS, WebView2 on Windows) — no plugin or
// capability needed, just the userAgent string.

const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";

export const isWindows = /Windows|Win64|Win32/i.test(ua);
export const isMac = /Macintosh|Mac OS X/i.test(ua) && !isWindows;

/** Human OS name, e.g. for "Launch when <OS> starts". */
export const osName = isWindows ? "Windows" : "macOS";

/** The OS file manager's name. */
export const fileManagerName = isWindows ? "File Explorer" : "Finder";

/** Verb + file-manager label for the reveal action, per-OS convention. */
export const revealLabel = isWindows ? "Show in Explorer" : "Reveal in Finder";

/** Modifier-key label for keyboard shortcuts. */
export const modKeyLabel = isWindows ? "Ctrl" : "⌘";
