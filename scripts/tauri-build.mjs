import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const preferredLocalKeyPath = join(homedir(), ".tauri", "app-orchestrator.key");
const legacyLocalKeyPath = join(homedir(), ".tauri", "local-project-orchestrator.key");
const localKeyPath = existsSync(preferredLocalKeyPath) ? preferredLocalKeyPath : legacyLocalKeyPath;
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

// On machines without the signing key (e.g. remote deploy targets like Zen),
// skip the updater tarball that would otherwise fail to sign. The .app and
// .dmg bundles still build normally; only the updater artifact is omitted.
// `--bundles app dmg` alone isn't enough because `createUpdaterArtifacts:true`
// in tauri.conf.json forces the updater tarball regardless, so we also pass
// `--config` to override that field for this invocation only.
const passthroughArgs = process.argv.slice(2);
const hasBundleFlag = passthroughArgs.some((arg) => arg === "--bundles" || arg === "-b");
const hasConfigFlag = passthroughArgs.some((arg) => arg === "--config" || arg === "-c");
if (!env.TAURI_SIGNING_PRIVATE_KEY) {
  if (!hasBundleFlag) {
    passthroughArgs.push("--bundles", "app", "dmg");
  }
  if (!hasConfigFlag) {
    passthroughArgs.push("--config", JSON.stringify({ bundle: { createUpdaterArtifacts: false } }));
  }
}

const result = spawnSync("npm", ["run", "tauri", "--", "build", ...passthroughArgs], {
  env,
  stdio: "inherit"
});

process.exit(result.status ?? 1);
