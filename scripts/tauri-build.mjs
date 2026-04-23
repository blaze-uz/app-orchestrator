import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const localKeyPath = join(homedir(), ".tauri", "local-project-orchestrator.key");
const env = { ...process.env };

if (!env.TAURI_SIGNING_PRIVATE_KEY && existsSync(localKeyPath)) {
  env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(localKeyPath, "utf8");
  env.TAURI_SIGNING_PRIVATE_KEY_PATH = localKeyPath;
}

if (env.TAURI_SIGNING_PRIVATE_KEY && env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD === undefined) {
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "";
}

if (process.platform === "darwin") {
  env.LANG = "en_US.UTF-8";
  env.LC_ALL = "en_US.UTF-8";
  env.LC_CTYPE = "en_US.UTF-8";
}

const result = spawnSync("npm", ["run", "tauri", "--", "build", ...process.argv.slice(2)], {
  env,
  stdio: "inherit"
});

process.exit(result.status ?? 1);
