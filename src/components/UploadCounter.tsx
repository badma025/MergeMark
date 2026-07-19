import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { Sparkles, KeyRound, Loader2 } from "lucide-react";

// ── Wire shape (mirrors `UsageStatus` in `src-tauri/src/commands.rs`) ─────────
//
// Kept in the frontend so the import site (App.tsx) doesn't have to
// re-declare the camelCase shape every time. If the Rust struct changes,
// this is the one place to update.
export interface UsageStatus {
  freeUploadsUsed: number;
  freeUploadsLimit: number;
  freeUploadsRemaining: number;
  byokKeyPresent: boolean;
  byokBaseUrl: string | null;
}

// Free-tier ceiling — also defined in `db::FREE_UPLOAD_LIMIT` on the backend.
// Kept here as a fallback in case the backend response is missing the field
// for any reason (older binary, schema drift, etc.).
const FREE_TIER_LIMIT = 3;

// ── Event-bus glue ───────────────────────────────────────────────────────────
//
// The hook and the badge live in different components. To let any caller
// (e.g. a future "Generate Worksheet" button, the ingestion dropzone, or a
// `sonner` toast) trigger a refresh without prop-drilling, we use a tiny
// window-level event:
//
//   * `useUploadCounter` listens for `mergemark:usage-changed` and re-fetches
//     whenever it fires.
//   * `notifyUsageChanged()` dispatches the event — call it from any code
//     that successfully invokes `generate_worksheet_from_pdf`.
//
// The event lives on `window` (rather than a module-level `EventTarget`)
// so it works correctly under React 18's StrictMode double-invocation and
// HMR re-mounts.
const USAGE_CHANGED_EVENT = "mergemark:usage-changed";

/**
 * Fire-and-forget helper. Call this immediately after a successful
 * `invoke("generate_worksheet_from_pdf", ...)` so the badge ticks down
 * without forcing the teacher to restart the app.
 *
 * Usage:
 *   await invoke("generate_worksheet_from_pdf", { filePath });
 *   notifyUsageChanged();
 */
export function notifyUsageChanged(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent(USAGE_CHANGED_EVENT));
}

// ── useUploadCounter hook ────────────────────────────────────────────────────
//
// Owns the live usage state in the parent component. The hook:
//   * fetches the status from the Rust backend on mount (via
//     `invoke("get_usage_status")`),
//   * re-fetches whenever `notifyUsageChanged()` is called,
//   * exposes a stable `refresh()` callback for callers that prefer to
//     trigger a re-fetch explicitly,
//   * derives `isByok` so the UI can switch its messaging when the free
//     tier is exhausted and we're running on the user's own key.
export function useUploadCounter() {
  const [status, setStatus] = useState<UsageStatus | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<UsageStatus>("get_usage_status");
      setStatus(next);
    } catch (err) {
      // Non-fatal — the badge just shows nothing until the next refresh.
      console.error("[UploadCounter] failed to fetch usage status:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();

    // Listen for the cross-component "please re-fetch" signal.
    if (typeof window === "undefined") return;
    const handler = () => {
      void refresh();
    };
    window.addEventListener(USAGE_CHANGED_EVENT, handler);
    return () => {
      window.removeEventListener(USAGE_CHANGED_EVENT, handler);
    };
  }, [refresh]);

  // The free tier is "exhausted" once the user has used all FREE_TIER_LIMIT
  // uploads. At that point the backend routes subsequent requests through
  // BYOK (if a key is on file) or returns `needs_byok` (if not). For the
  // badge messaging we treat the app as "in BYOK mode" any time the free
  // counter is at the cap, regardless of whether a key is stored, because
  // the user's next click will go through their key (or prompt for one).
  const isByok = status != null && status.freeUploadsUsed >= FREE_TIER_LIMIT;

  return { status, refresh, isByok, loading };
}

// ── <UploadCounter /> ────────────────────────────────────────────────────────
//
// A small, non-intrusive badge intended to live in the top tab bar. It
// shows one of three states:
//
//   1. `loading=true` and no status yet     → "Loading…" pill.
//   2. `isByok === true`                    → "Active Billing: Personal API Key"
//   3. otherwise                            → "Free Uploads Left: X / 3"
//
// Colour cues follow the existing UI palette: secondary for the "active"
// state, a yellow-ish muted tone for the warning "0 left" state, and
// outline for everything else. The `byokKeyPresent` flag from the
// backend is what switches the variant, so a user with no BYOK key still
// sees the free-tier text but in a destructive tone once they're capped.
export interface UploadCounterProps {
  status: UsageStatus | null;
  loading?: boolean;
  className?: string;
}

export function UploadCounter({ status, loading = false, className }: UploadCounterProps) {
  if (loading && status == null) {
    return (
      <Badge
        variant="outline"
        className={cn("gap-1.5 font-normal", className)}
        aria-label="Loading upload status"
      >
        <Loader2 className="size-3 animate-spin" aria-hidden />
        <span>Loading…</span>
      </Badge>
    );
  }

  if (status == null) {
    // Backend unreachable / invoke failed; render nothing rather than a
    // broken-looking counter.
    return null;
  }

  // Compute the displayed numbers defensively. The backend always returns
  // these as i64, but we clamp on the frontend so a future schema change
  // can't surface a negative count to the teacher.
  const used = Math.max(0, status.freeUploadsUsed);
  const limit = status.freeUploadsLimit > 0 ? status.freeUploadsLimit : FREE_TIER_LIMIT;
  const remaining = Math.max(0, status.freeUploadsRemaining);

  // ── State 1: free tier exhausted ──────────────────────────────────────
  if (used >= limit) {
    // If the user has stored a key, we're actively running on their quota.
    // If they don't, the next click will prompt them for one — so we
    // still treat this as "BYOK mode" but call it out as a warning so
    // they know what's coming.
    const hasKey = status.byokKeyPresent === true;
    return (
      <Badge
        variant={hasKey ? "secondary" : "destructive"}
        className={cn("gap-1.5 font-medium", className)}
        aria-label="Active billing: Personal API Key"
        title={
          hasKey
            ? "Free tier exhausted — running on your personal API key"
            : "Free tier exhausted — add a personal API key in Settings to keep going"
        }
      >
        <KeyRound className="size-3" aria-hidden />
        <span>
          Active Billing: {hasKey ? "Personal API Key" : "Add API Key"}
        </span>
      </Badge>
    );
  }

  // ── State 2: free tier in use ────────────────────────────────────────
  // Switch to a slightly more attention-grabbing tone when the user has
  // burned through most of the free credits (1 left, 0 left being the
  // most natural "running out" cues).
  const variant = remaining === 0 ? "destructive" : remaining === 1 ? "secondary" : "outline";
  const toneClass =
    remaining === 0
      ? "font-semibold"
      : remaining === 1
      ? "text-yellow-500 dark:text-yellow-400 font-medium"
      : "text-muted-foreground";

  return (
    <Badge
      variant={variant}
      className={cn("gap-1.5 font-normal", toneClass, className)}
      aria-label={`Free uploads left: ${remaining} of ${limit}`}
      title={`${remaining} of ${limit} free uploads remaining`}
    >
      <Sparkles className="size-3" aria-hidden />
      <span>Free Uploads Left: {remaining} / {limit}</span>
    </Badge>
  );
}
