import { existsSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const appName = "Local Project Orchestrator.app";
const installedAppPath = join("/Applications", appName);
const builtAppPath = join(process.cwd(), "src-tauri", "target", "release", "bundle", "macos", appName);
const appPath = existsSync(installedAppPath) ? installedAppPath : builtAppPath;

if (process.platform !== "darwin") {
  console.error("desktop:open is only supported on macOS.");
  process.exit(1);
}

if (!existsSync(appPath)) {
  console.error("No installed or built app was found. Run `npm run desktop:install` first.");
  process.exit(1);
}

const result = spawnSync("open", [appPath], { stdio: "inherit" });
process.exit(result.status ?? 1);
