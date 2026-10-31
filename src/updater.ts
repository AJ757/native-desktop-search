import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask } from "@tauri-apps/plugin-dialog";

export async function checkForUpdates() {
  try {
    const update = await check();
    if (!update) return;

    const install = await ask(`Version ${update.version} is available. Install now?`, {
      title: "Update available",
      kind: "info",
      okLabel: "Install",
      cancelLabel: "Later",
    });
    if (!install) return;

    await update.downloadAndInstall();
    await relaunch();
  } catch (err) {
    console.error("Update check failed:", err);
  }
}

export function startUpdateChecks() {
  if (!import.meta.env.PROD) return; // skip during `tauri dev`
  void checkForUpdates();
  setInterval(checkForUpdates, 6 * 60 * 60 * 1000);
}
