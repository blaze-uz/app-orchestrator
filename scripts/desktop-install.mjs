import { existsSync, rmSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const appName = "Local Project Orchestrator.app";
const bundleIdentifier = "dev.local-project-orchestrator.app";
const builtAppPath = join(process.cwd(), "src-tauri", "target", "release", "bundle", "macos", appName);
const installedAppPath = join("/Applications", appName);

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: options.quiet ? "ignore" : "inherit" });
  if (!options.allowFailure && result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  return result.status === 0;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isInstalledAppRunning() {
  const result = spawnSync("pgrep", ["-f", `${appName}/Contents/MacOS/local-project-orchestrator`], {
    stdio: "ignore"
  });
  return result.status === 0;
}

async function waitForInstalledAppToQuit() {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    if (!isInstalledAppRunning()) return true;
    await sleep(500);
  }
  return !isInstalledAppRunning();
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

run("osascript", ["-e", `tell application id "${bundleIdentifier}" to quit`], {
  allowFailure: true,
  quiet: true
});
if (!(await waitForInstalledAppToQuit())) {
  console.error("Local Project Orchestrator is still running. Quit it and rerun `npm run desktop:install`.");
  process.exit(1);
}

try {
  rmSync(installedAppPath, { recursive: true, force: true });
} catch (error) {
  console.error(`Unable to replace ${installedAppPath}. You may need write access to /Applications.`);
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}

run("ditto", [builtAppPath, installedAppPath]);
run("open", [installedAppPath]);
