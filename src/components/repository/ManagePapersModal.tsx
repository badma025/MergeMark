import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Trash2, Loader2, AlertTriangle } from "lucide-react";

interface ManagePapersModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPaperDeleted: () => void;
}

export function ManagePapersModal({ open, onOpenChange, onPaperDeleted }: ManagePapersModalProps) {
  const [papers, setPapers] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [deletingPaper, setDeletingPaper] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      loadPapers();
      setConfirmDelete(null);
    }
  }, [open]);

  async function loadPapers() {
    setLoading(true);
    try {
      const names = await invoke<string[]>("get_paper_names");
      setPapers(names);
    } catch (err) {
      toast.error("Failed to load imported papers", { description: String(err) });
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete(paperName: string) {
    setDeletingPaper(paperName);
    try {
      const deletedRows = await invoke<number>("delete_questions_by_paper", { paperName });
      toast.success(`Successfully removed "${paperName}"`, {
        description: `Deleted ${deletedRows} associated questions and mark scheme answers.`,
      });
      setPapers(prev => prev.filter(p => p !== paperName));
      onPaperDeleted();
      setConfirmDelete(null);
    } catch (err) {
      toast.error(`Failed to remove "${paperName}"`, { description: String(err) });
    } finally {
      setDeletingPaper(null);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!deletingPaper) onOpenChange(v); }}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Manage Imported PDFs</DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-4 py-4 min-h-[200px] max-h-[60vh] overflow-y-auto pr-2">
          {loading ? (
            <div className="flex items-center justify-center h-full min-h-[150px]">
              <Loader2 className="size-6 animate-spin text-muted-foreground" />
            </div>
          ) : papers.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full min-h-[150px] text-center text-muted-foreground">
              <p>No imported PDFs found.</p>
              <p className="text-xs mt-1">Manually created questions are preserved automatically.</p>
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              {papers.map((paper) => {
                const isConfirming = confirmDelete === paper;
                const isDeleting = deletingPaper === paper;
                
                return (
                  <div 
                    key={paper} 
                    className="flex flex-col gap-2 p-3 rounded-lg border border-border bg-card shadow-sm transition-all"
                  >
                    <div className="flex items-center justify-between gap-3">
                      <span className="font-medium text-sm text-foreground truncate" title={paper}>
                        {paper}
                      </span>
                      
                      {!isConfirming && (
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-muted-foreground hover:text-destructive hover:bg-destructive/10 shrink-0"
                          onClick={() => setConfirmDelete(paper)}
                          disabled={deletingPaper !== null}
                          aria-label={`Remove ${paper}`}
                        >
                          <Trash2 className="size-4" />
                        </Button>
                      )}
                    </div>

                    {isConfirming && (
                      <div className="flex flex-col gap-3 mt-2 pt-3 border-t border-destructive/20 bg-destructive/5 -mx-3 -mb-3 p-3 rounded-b-lg">
                        <div className="flex gap-2 text-sm text-destructive font-medium">
                          <AlertTriangle className="size-4 shrink-0 mt-0.5" />
                          <p>
                            Are you sure? This will permanently delete all questions and mark schemes associated with this paper.
                          </p>
                        </div>
                        <div className="flex items-center justify-end gap-2">
                          <Button 
                            variant="ghost" 
                            size="sm" 
                            onClick={() => setConfirmDelete(null)}
                            disabled={isDeleting}
                            className="h-8 text-xs"
                          >
                            Cancel
                          </Button>
                          <Button 
                            variant="destructive" 
                            size="sm"
                            onClick={() => handleDelete(paper)}
                            disabled={isDeleting}
                            className="h-8 text-xs gap-1.5"
                          >
                            {isDeleting ? (
                              <><Loader2 className="size-3 animate-spin" /> Deleting...</>
                            ) : (
                              "Yes, Remove Everything"
                            )}
                          </Button>
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
