import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { preprocessMath } from "@/components/repository/QuestionCard";
import { Loader2 } from "lucide-react";

export interface ProposedMapping {
  questionId: string;
  rawContent: string;
  proposedAnswer: string;
}

interface ReviewSyncModalProps {
  mappings: ProposedMapping[] | null;
  onClose: () => void;
  onSuccess: () => void;
}

export function ReviewSyncModal({ mappings, onClose, onSuccess }: ReviewSyncModalProps) {
  const [isCommitting, setIsCommitting] = useState(false);
  const [localMappings, setLocalMappings] = useState<ProposedMapping[]>([]);

  useEffect(() => {
    if (mappings) {
      setLocalMappings(mappings);
    }
  }, [mappings]);

  function handleSwap(currentIndex: number, targetIndex: number) {
    if (currentIndex === targetIndex) return;
    setLocalMappings((prev) => {
      const next = [...prev];
      
      // Create fresh objects to ensure React detects the update!
      const currentItem = { ...next[currentIndex] };
      const targetItem = { ...next[targetIndex] };
      
      // Swap ONLY the proposedAnswer fields
      const tempAnswer = currentItem.proposedAnswer;
      currentItem.proposedAnswer = targetItem.proposedAnswer;
      targetItem.proposedAnswer = tempAnswer;
      
      next[currentIndex] = currentItem;
      next[targetIndex] = targetItem;
      
      return next;
    });
  }

  if (!mappings) return null;

  async function handleCommit() {
    setIsCommitting(true);
    try {
      await invoke("commit_mark_schemes", { mappings: localMappings });
      toast.success("Mappings successfully committed to the repository!");
      onSuccess();
    } catch (err) {
      toast.error("Failed to commit mappings", { description: String(err) });
    } finally {
      setIsCommitting(false);
    }
  }

  return (
    <Dialog open={mappings !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[95vw] w-[95vw] h-[90vh] max-w-none flex flex-col">
        <DialogHeader>
          <DialogTitle>Review Mappings</DialogTitle>
          <DialogDescription>
            Review the extracted answers below. If any answers are misaligned with their questions, you can swap them using the dropdown.
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto min-h-0 bg-muted/20 rounded-md border p-4">
          {localMappings.length === 0 ? (
            <div className="text-center text-muted-foreground p-8">
              No new mappings were proposed. Either all questions have answers, or none matched.
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              {localMappings.map((m, currentIndex) => (
                <div key={m.questionId} className="flex gap-4 border-b pb-4 last:border-0 last:pb-0">
                  <div className="flex-1 prose prose-sm dark:prose-invert border rounded bg-background/50 p-3 opacity-90 overflow-x-auto whitespace-normal break-words">
                    <div className="text-xs font-semibold text-muted-foreground mb-2 uppercase tracking-wider">
                      Question {currentIndex + 1}
                    </div>
                    <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                      {preprocessMath(m.rawContent)}
                    </ReactMarkdown>
                  </div>

                  <div className="flex-1 flex flex-col prose prose-sm dark:prose-invert border rounded bg-background p-3 relative overflow-x-auto whitespace-normal break-words">
                    <div className="flex items-center justify-between gap-2 mb-2">
                      <div className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                        Proposed Answer
                      </div>
                      <div className="flex items-center gap-2">
                        <label htmlFor={`swap-${currentIndex}`} className="text-xs font-medium text-muted-foreground">
                          Match to:
                        </label>
                        <select
                          id={`swap-${currentIndex}`}
                          value={currentIndex}
                          onChange={(e) => handleSwap(currentIndex, parseInt(e.target.value))}
                          className="text-xs bg-background border border-border rounded px-2 py-1 focus:ring-1 focus:ring-primary focus:outline-none"
                        >
                          {localMappings.map((_, i) => (
                            <option key={i} value={i}>
                              Question {i + 1}
                            </option>
                          ))}
                        </select>
                      </div>
                    </div>
                    <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                      {preprocessMath(m.proposedAnswer)}
                    </ReactMarkdown>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        <DialogFooter className="mt-4 border-t pt-4">
          <Button variant="outline" onClick={onClose} disabled={isCommitting}>Discard</Button>
          <Button onClick={handleCommit} disabled={isCommitting || localMappings.length === 0}>
            {isCommitting ? <Loader2 className="animate-spin size-4 mr-2" /> : null}
            Commit to Repository
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
