import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
const semverPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

if (!version || !semverPattern.test(version)) {
  console.error("Usage: npm run version:set -- <semver>");
  process.exit(1);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, data) {
  writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);
}

function replaceVersion(path, pattern) {
  const content = readFileSync(path, "utf8");
  if (!pattern.test(content)) {
    throw new Error(`Unable to update version in ${path}`);
  }
  const updated = content.replace(pattern, `$1${version}$2`);
  writeFileSync(path, updated);
}

const packageJson = readJson("package.json");
packageJson.version = version;
writeJson("package.json", packageJson);

const packageLock = readJson("package-lock.json");
packageLock.version = version;
if (packageLock.packages?.[""]) {
  packageLock.packages[""].version = version;
}
writeJson("package-lock.json", packageLock);

replaceVersion("src-tauri/Cargo.toml", /^(version\s*=\s*")[^"]+(")$/m);
replaceVersion("src-tauri/tauri.conf.json", /^(\s*"version"\s*:\s*")[^"]+(")/m);

console.log(`Updated app version to ${version}`);
