import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";
import { spawnSync } from "node:child_process";

const candidateKeyPaths = [
  join(homedir(), ".tauri", "karvon.key"),
  join(homedir(), ".tauri", "app-orchestrator.key"),
  join(homedir(), ".tauri", "local-project-orchestrator.key"),
];
const localKeyPath = candidateKeyPaths.find((p) => existsSync(p)) ?? candidateKeyPaths[0];
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

// Make cargo (installed by rustup) discoverable even when PATH is minimal — e.g.
// a GUI-launched build or a self-deploy pipeline. The npm `tauri` script no
// longer hard-codes this so it stays cross-platform; we add it here instead.
const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin) && !(env.PATH ?? "").split(delimiter).includes(cargoBin)) {
  env.PATH = `${cargoBin}${delimiter}${env.PATH ?? ""}`;
}

// On machines without the signing key (e.g. remote deploy targets like Zen),
// skip the updater artifact that would otherwise fail to sign. The installer
// bundles still build normally; only the updater artifact is omitted.
// Naming the bundles alone isn't enough because `createUpdaterArtifacts:true`
// in tauri.conf.json forces the updater artifact regardless, so we also pass
// `--config` to override that field for this invocation only.
const passthroughArgs = process.argv.slice(2);
const hasBundleFlag = passthroughArgs.some((arg) => arg === "--bundles" || arg === "-b");
const hasConfigFlag = passthroughArgs.some((arg) => arg === "--config" || arg === "-c");
// Per-OS bundle types (the macOS `app`/`dmg` bundlers don't exist on Windows).
const fallbackBundles = process.platform === "win32" ? ["nsis"] : ["app", "dmg"];
if (!env.TAURI_SIGNING_PRIVATE_KEY) {
  if (!hasBundleFlag) {
    passthroughArgs.push("--bundles", ...fallbackBundles);
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
