import { check } from '@tauri-apps/plugin-updater';
import { ask } from '@tauri-apps/plugin-dialog';
import { relaunch } from '@tauri-apps/plugin-process';

export async function checkForAppUpdates() {
  try {
    const update = await check();

    if (update) {
      const wantsUpdate = await ask(
        `MergeMark ${update.version} is available!\n\nRelease notes:\n${update.body}\n\nDo you want to install it now?`,
        { title: 'Update Available', kind: 'info' }
      );

      if (wantsUpdate) {
        await update.downloadAndInstall();
        await relaunch();
      }
    }
  } catch (error) {
    console.error("Failed to check for updates:", error);
  }
}
