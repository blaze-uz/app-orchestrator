import { existsSync } from "node:fs";
import { join } from "node:path";
import { spawnSync, spawn } from "node:child_process";

const appName = "Karvon.app";
const legacyAppName = "App Orchestrator.app";
const bundleIdentifier = "uz.blaze.karvon";
const legacyBundleIdentifier = "uz.blaze.app-orchestrator";
const builtAppPath = join(process.cwd(), "src-tauri", "target", "release", "bundle", "macos", appName);
const installedAppPath = join("/Applications", appName);
const legacyInstalledAppPath = join("/Applications", legacyAppName);

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: options.quiet ? "ignore" : "inherit" });
  if (!options.allowFailure && result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  return result.status === 0;
}

if (process.platform !== "darwin") {
  console.error("desktop:install is only supported on macOS.");
  process.exit(1);
}

run("npm", ["run", "desktop:build"]);

if (!existsSync(builtAppPath)) {
  console.error(`Built app was not found at ${builtAppPath}`);
  process.exit(1);
}

// The swap (quit running orchestrator → replace bundle → relaunch) must NOT
// happen inline: when the orchestrator deploys ITSELF, this script runs as a
// child of the orchestrator's deploy pipeline. Quitting the app here would kill
// that pipeline before it records the deploy as succeeded, so the next
// auto-deploy poll sees the project as out-of-sync and triggers ANOTHER build —
// an infinite self-redeploy loop that stacks concurrent `tauri build`s.
//
// Instead we hand the swap to a detached, session-leader child (setsid via
// `nohup`/`unref`) that waits a few seconds — long enough for the deploy
// pipeline to finish and persist `lastSucceededCommit` — then quits the old app,
// replaces the bundle, and relaunches. The pipeline sees this script exit 0
// immediately, records success, and only then does the new app come up.
const swap = [
  // Give the (possibly self-)deploy pipeline time to record success first.
  "sleep 6",
  // Ask both current and legacy bundle ids to quit, then wait up to ~10s.
  `osascript -e 'tell application id "${bundleIdentifier}" to quit' >/dev/null 2>&1 || true`,
  `osascript -e 'tell application id "${legacyBundleIdentifier}" to quit' >/dev/null 2>&1 || true`,
  `for i in $(seq 1 20); do pgrep -f '${appName}/Contents/MacOS/karvon|${legacyAppName}/Contents/MacOS/app-orchestrator' >/dev/null 2>&1 || break; sleep 0.5; done`,
  // Replace the installed bundle (force, both current + legacy paths).
  `rm -rf ${JSON.stringify(installedAppPath)} ${JSON.stringify(legacyInstalledAppPath)}`,
  `ditto ${JSON.stringify(builtAppPath)} ${JSON.stringify(installedAppPath)}`,
  `open ${JSON.stringify(installedAppPath)}`,
].join("\n");

const child = spawn("/bin/bash", ["-lc", swap], {
  detached: true,
  stdio: "ignore",
});
child.unref();

console.log("Desktop bundle built. Detached relaunch scheduled (~6s) so the deploy can record success first.");
process.exit(0);
