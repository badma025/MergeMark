import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask } from "@tauri-apps/plugin-dialog";

/**
 * Updates come from the project's GitHub releases: the app asks for the
 * newest release's details and installs it only when its signature is
 * MergeMark's. It is the one request the app makes; imports never leave the
 * computer.
 */

const AUTO_CHECK_KEY = "mergemark.checkForUpdates";

/** Whether MergeMark checks for an update when it starts (on unless turned off). */
export function autoCheckEnabled(): boolean {
  try {
    return localStorage.getItem(AUTO_CHECK_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setAutoCheck(on: boolean): void {
  try {
    localStorage.setItem(AUTO_CHECK_KEY, on ? "on" : "off");
  } catch {
    // Not remembered; the default (on) applies next time.
  }
}

export type UpdateOutcome = "up-to-date" | "later" | "installing";

/**
 * Looks for a newer release and offers to install it, then restarts into
 * it. Throws when the check cannot be made (offline, or GitHub unreachable).
 */
export async function checkForUpdates(): Promise<UpdateOutcome> {
  const update = await check();
  if (!update) return "up-to-date";
  const notes = update.body?.trim() ? `\n\n${update.body.trim()}` : "";
  const install = await ask(
    `MergeMark ${update.version} is available (you have ${update.currentVersion}).${notes}\n\nYour library is backed up when the new version first opens it.`,
    { title: "Update available", kind: "info", okLabel: "Install and restart", cancelLabel: "Later" },
  );
  if (!install) return "later";
  await update.downloadAndInstall();
  await relaunch();
  return "installing";
}

/** The check at start-up: silent unless there is an update to offer. */
export async function checkForUpdatesAtStart(): Promise<void> {
  if (!autoCheckEnabled()) return;
  try {
    await checkForUpdates();
  } catch (error) {
    console.warn("Update check failed:", error);
  }
}
