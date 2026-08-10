"use client";

import { useEffect, useState } from "react";
import { Download, AlertTriangle, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";

interface UpdateInfo {
  version: string;
  body: string;
  date: string;
  current_version: string;
}

interface UpdateProgress {
  downloaded: number;
  total: number;
}

export function UpdateChecker() {
  const [installing, setInstalling] = useState(false);

  const checkForUpdate = async () => {
    try {
      const update = await check();
      if (update) {
        // Show toast notification
        toast("Update Available", {
          description: `Version ${update.version} is ready to install.`,
          action: {
            label: installing ? "Installing..." : "Update Now",
            onClick: handleInstallUpdate,
          },
          duration: 0, // Don't auto-dismiss
        });
      }
    } catch (error) {
      console.error("Failed to check for updates:", error);
    }
  };

  const handleInstallUpdate = async () => {
    setInstalling(true);

    try {
      // Listen for progress events from backend
      let unlisten: (() => void) | null = null;
      try {
        unlisten = await invoke("listen", {
          event: "update-progress",
          handler: () => {
            // We don't use progress in toast-based approach
          },
        });
      } catch (e) {
        console.warn("Could not listen for update progress:", e);
      }

      const update = await check();
      if (update) {
        await update.downloadAndInstall(
          (event) => {
            // Could emit progress events here if needed
            if (event.event === "Started" && event.data.contentLength) {
              console.log(`Download started: ${event.data.contentLength} bytes`);
            }
          }
        );
      }

      if (unlisten) unlisten();

      toast.success("Update Installed", {
        description: "The app will restart to apply the update.",
      });

      // The app will restart automatically after install
    } catch (error) {
      console.error("Failed to install update:", error);
      toast.error("Update Failed", {
        description: "Failed to install update. Please try again later.",
      });
    } finally {
      setInstalling(false);
    }
  };

  // Check on mount
  useEffect(() => {
    checkForUpdate();

    // Check every 4 hours (14400000 ms)
    const interval = setInterval(checkForUpdate, 4 * 60 * 60 * 1000);

    return () => clearInterval(interval);
  }, []);

  // Don't render anything - this component works via toasts
  return null;
}

export function UpdateButton() {
  const [checking, setChecking] = useState(false);
  const [updateAvailable, setUpdateAvailable] = useState<UpdateInfo | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);

  const checkForUpdate = async () => {
    setChecking(true);
    try {
      const update = await check();
      if (update) {
        setUpdateAvailable({
          version: update.version,
          body: update.body || "",
          date: update.date || "",
          current_version: update.currentVersion,
        });
        toast("Update Available", {
          description: `Version ${update.version} is ready to install.`,
        });
      } else {
        toast.success("Up to Date", {
          description: "You're running the latest version.",
        });
      }
    } catch (error) {
      console.error("Failed to check for updates:", error);
      toast.error("Check Failed", {
        description: "Unable to check for updates. Please try again.",
      });
    } finally {
      setChecking(false);
    }
  };

  const handleInstallUpdate = async () => {
    setInstalling(true);
    setProgress({ downloaded: 0, total: 0 });

    try {
      let unlisten: (() => void) | null = null;
      try {
        unlisten = await invoke("listen", {
          event: "update-progress",
          handler: (event: { payload: UpdateProgress }) => {
            setProgress(event.payload);
          },
        });
      } catch (e) {
        console.warn("Could not listen for update progress:", e);
      }

      const update = await check();
      if (update) {
        await update.downloadAndInstall(
          (event) => {
            if (event.event === "Progress") {
              setProgress({
                downloaded: event.data.chunkLength,
                total: progress?.total || 0,
              });
            } else if (event.event === "Started" && event.data.contentLength) {
              setProgress({
                downloaded: 0,
                total: event.data.contentLength,
              });
            }
          }
        );
      }

      if (unlisten) unlisten();

      toast.success("Update Installed", {
        description: "The app will restart to apply the update.",
      });
    } catch (error) {
      console.error("Failed to install update:", error);
      toast.error("Update Failed", {
        description: "Failed to install update. Please try again later.",
      });
    } finally {
      setInstalling(false);
      setProgress(null);
      setUpdateAvailable(null);
    }
  };

  if (!updateAvailable) {
    return (
      <Button
        variant="outline"
        size="sm"
        onClick={checkForUpdate}
        disabled={checking}
        className="gap-2"
      >
        {checking ? (
          <>
            <RefreshCw className="h-4 w-4 animate-spin" />
            Checking...
          </>
        ) : (
          <>
            <RefreshCw className="h-4 w-4" />
            Check for Updates
          </>
        )}
      </Button>
    );
  }

  const percent = progress && progress.total > 0
    ? Math.round((progress.downloaded / progress.total) * 100)
    : 0;

  return (
    <div className="flex items-center gap-2">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <AlertTriangle className="h-4 w-4 text-amber-500" />
        <span>v{updateAvailable.version} available</span>
      </div>
      <Button
        variant="default"
        size="sm"
        onClick={handleInstallUpdate}
        disabled={installing}
        className="gap-2"
      >
        {installing ? (
          <>
            <RefreshCw className="h-4 w-4 animate-spin" />
            {percent > 0 ? `Installing... ${percent}%` : "Installing..."}
          </>
        ) : (
          <>
            <Download className="h-4 w-4" />
            Install Update
          </>
        )}
      </Button>
    </div>
  );
}