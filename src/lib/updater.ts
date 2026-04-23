import { getVersion } from "@tauri-apps/api/app";
import { confirm } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { api } from "./api";

export type { Update };
export const canUseAppUpdater = api.isTauri;

export interface UpdateProgress {
  downloadedBytes: number;
  totalBytes?: number;
  percent?: number;
}

export async function getCurrentAppVersion() {
  if (!api.isTauri) return __APP_VERSION__;
  return getVersion();
}

export async function checkForAppUpdate() {
  if (!api.isTauri) {
    throw new Error("Update checks are only available in the desktop app.");
  }
  return check({ timeout: 30000 });
}

export async function confirmUpdateInstall(version: string) {
  if (!api.isTauri) return window.confirm(`Install version ${version} and relaunch the app?`);
  return confirm(`Install version ${version} now? Running processes will be stopped while the app relaunches.`, {
    title: "Install update",
    kind: "warning"
  });
}

export async function installAppUpdate(update: Update, onProgress: (progress: UpdateProgress) => void) {
  let downloadedBytes = 0;
  let totalBytes: number | undefined;

  await update.downloadAndInstall((event: DownloadEvent) => {
    if (event.event === "Started") {
      downloadedBytes = 0;
      totalBytes = event.data.contentLength;
    }

    if (event.event === "Progress") {
      downloadedBytes += event.data.chunkLength;
    }

    if (event.event === "Finished") {
      downloadedBytes = totalBytes ?? downloadedBytes;
    }

    onProgress({
      downloadedBytes,
      totalBytes,
      percent: totalBytes ? Math.min(100, Math.round((downloadedBytes / totalBytes) * 100)) : undefined
    });
  });

  await relaunch();
}
