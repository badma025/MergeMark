import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { autoCheckEnabled, checkForUpdates, setAutoCheck } from "@/lib/updates";

/** The app's version, a check for a newer one, and whether to check at start-up. */
export function UpdatesCard() {
  const [version, setVersion] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [auto, setAuto] = useState(autoCheckEnabled());

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  async function handleCheck() {
    setChecking(true);
    try {
      const outcome = await checkForUpdates();
      if (outcome === "up-to-date") toast.success("MergeMark is up to date");
    } catch (err) {
      toast.error("Could not check for updates", {
        description: `MergeMark asks its GitHub releases page; check your connection. (${String(err)})`,
      });
    } finally {
      setChecking(false);
    }
  }

  return (
    <div className="flex flex-col gap-4 rounded-xl border border-border/70 bg-card p-6 shadow-xs">
      <div className="border-b border-border/60 pb-3">
        <h2 className="text-sm font-bold flex items-center gap-2 text-foreground">
          <RefreshCw className="size-4 text-primary" />
          Updates
        </h2>
        <p className="text-xs text-muted-foreground mt-0.5">
          {version ? `You have MergeMark ${version}. ` : ""}
          Updates come from MergeMark's GitHub releases and install only when signed by MergeMark. Checking is the one
          request MergeMark makes; your papers never leave this computer.
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-4">
        <Button variant="outline" className="gap-2 text-xs font-medium" onClick={handleCheck} disabled={checking}>
          <RefreshCw className={checking ? "size-3.5 animate-spin" : "size-3.5"} />
          <span>{checking ? "Checking..." : "Check for updates"}</span>
        </Button>
        <label className="flex items-center gap-2 text-xs text-foreground select-none cursor-pointer">
          <input
            type="checkbox"
            checked={auto}
            onChange={(e) => {
              setAuto(e.target.checked);
              setAutoCheck(e.target.checked);
            }}
            className="size-3.5 accent-primary"
          />
          Check when MergeMark starts
        </label>
      </div>
    </div>
  );
}
