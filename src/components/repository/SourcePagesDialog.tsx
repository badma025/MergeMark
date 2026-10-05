import { useEffect, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderSearch, Loader2, ZoomIn, ZoomOut } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { pagesLabel, sourceProblem, type SourceView } from "@/lib/source-pages";

/**
 * The pages of the paper a card was imported from, rendered at 300 DPI, to
 * check the card against. A paper that moved or changed is said to be so,
 * never shown in its place; the user can find the file again.
 */
export function SourcePagesDialog({
  id,
  title,
  isOpen,
  onClose,
}: {
  id: string;
  title: string;
  isOpen: boolean;
  onClose: () => void;
}) {
  const [view, setView] = useState<SourceView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [locating, setLocating] = useState(false);
  const [scale, setScale] = useState(1);

  useEffect(() => {
    if (!isOpen) return;
    let live = true;
    setView(null);
    setError(null);
    invoke<SourceView>("show_source_pages", { id })
      .then((found) => live && setView(found))
      .catch((err) => live && setError(String(err)));
    return () => {
      live = false;
    };
  }, [isOpen, id]);

  async function locate() {
    const picked = await open({ multiple: false, directory: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
    if (typeof picked !== "string") return;
    setLocating(true);
    try {
      setView(await invoke<SourceView>("relocate_source", { id, path: picked }));
    } catch (err) {
      toast.error("That file is not this card's paper", { description: String(err) });
    } finally {
      setLocating(false);
    }
  }

  const problem = view ? sourceProblem(view) : null;
  const subtitle = view?.status === "pages" ? `${pagesLabel(view.firstPage, view.lastPage)} · ${view.path}` : "The paper this card was imported from";

  return (
    <Dialog open={isOpen} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-5xl w-[95vw] max-h-[90vh] p-0 overflow-hidden flex flex-col">
        <DialogHeader className="px-6 py-4 border-b border-border/60 flex flex-row items-center justify-between space-y-0 gap-4">
          <div className="min-w-0">
            <DialogTitle className="text-base font-semibold">{title}</DialogTitle>
            <p className="text-xs text-muted-foreground mt-0.5 truncate" title={subtitle}>
              {subtitle}
            </p>
          </div>
          {view?.status === "pages" && (
            <div className="flex items-center gap-1.5 mr-6 shrink-0">
              <Button variant="outline" size="icon" className="size-8" onClick={() => setScale((s) => Math.max(s - 0.25, 0.5))} title="Zoom out">
                <ZoomOut className="size-4" />
              </Button>
              <Button variant="outline" size="sm" className="h-8 px-2.5 text-xs font-mono" onClick={() => setScale(1)} title="Fit to width">
                {Math.round(scale * 100)}%
              </Button>
              <Button variant="outline" size="icon" className="size-8" onClick={() => setScale((s) => Math.min(s + 0.25, 3))} title="Zoom in">
                <ZoomIn className="size-4" />
              </Button>
            </div>
          )}
        </DialogHeader>
        <div className="flex-1 min-h-0 overflow-auto bg-black/40 p-6">
          {!view && !error && (
            <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" />
              Rendering the paper's pages...
            </div>
          )}
          {error && <p className="py-16 text-center text-sm text-destructive">{error}</p>}
          {problem && (
            <div className="mx-auto flex max-w-xl flex-col items-center gap-4 py-12 text-center">
              <p className="text-sm text-foreground">{problem.message}</p>
              {problem.canLocate && (
                <Button onClick={locate} disabled={locating} className="gap-2">
                  {locating ? <Loader2 className="size-4 animate-spin" /> : <FolderSearch className="size-4" />}
                  Find the PDF...
                </Button>
              )}
            </div>
          )}
          {view?.status === "pages" && (
            <div className="flex flex-col items-center gap-6">
              {view.images.map((image, k) => (
                <figure key={image} className="flex flex-col items-center gap-1.5" style={{ width: `${scale * 100}%` }}>
                  <img
                    src={convertFileSrc(image)}
                    alt={`Page ${view.firstPage + k} of the paper`}
                    className="w-full rounded-md bg-white shadow-lg ring-1 ring-border/40"
                  />
                  <figcaption className="text-xs text-muted-foreground">Page {view.firstPage + k}</figcaption>
                </figure>
              ))}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
