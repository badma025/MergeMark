import { useState, useEffect } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
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
  paperName: string;
}

interface ReviewSyncModalProps {
  mappings: ProposedMapping[] | null;
  onClose: () => void;
  onSuccess: () => void;
}

/**
 * Extract the leading integer question number from the raw question content.
 * Handles formats like: "1. Find...", "Question 1\n...", "1) Find...", "Q1 ..."
 * Falls back to `fallback` (the array index) if no number is found.
 */
function extractQuestionNumber(rawContent: string, fallback: number): number {
  const match = rawContent.trim().match(/^(?:Question\s+|Q\.?\s*)?(\d+)/i);
  return match ? parseInt(match[1], 10) : fallback;
}

export function ReviewSyncModal({ mappings, onClose, onSuccess }: ReviewSyncModalProps) {
  const [isCommitting, setIsCommitting] = useState(false);
  const [localMappings, setLocalMappings] = useState<ProposedMapping[]>([]);

  useEffect(() => {
    if (mappings) {
      setLocalMappings(mappings);
    }
  }, [mappings]);

  /**
   * Swap the proposedAnswer of the mapping at `currentIndex` with the mapping
   * whose extracted question number equals `targetQNum` AND whose paperName
   * matches the current mapping's paperName (prevents cross-paper swaps).
   */
  function handleSwap(currentIndex: number, targetQNum: number) {
    const currentPaperName = localMappings[currentIndex].paperName;
    const targetIndex = localMappings.findIndex(
      (m, idx) =>
        extractQuestionNumber(m.rawContent, idx + 1) === targetQNum &&
        m.paperName === currentPaperName
    );
    if (targetIndex === -1 || currentIndex === targetIndex) return;

    setLocalMappings((prev) => {
      const next = [...prev];
      const currentItem = { ...next[currentIndex] };
      const targetItem = { ...next[targetIndex] };

      // Swap ONLY the proposedAnswer fields; questionId stays anchored to rawContent.
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
            Review the extracted answers below. Each row is matched by the question number parsed
            directly from the mark scheme — not by position. If any answers are misaligned, use the
            dropdown to re-assign them.
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto min-h-0 bg-muted/20 rounded-md border p-4">
          {localMappings.length === 0 ? (
            <div className="text-center text-muted-foreground p-8">
              No new mappings were proposed. Either all questions have answers, or none matched.
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              {localMappings.map((m, currentIndex) => {
                // Derive the display number from the content itself, not the array index.
                const qNum = extractQuestionNumber(m.rawContent, currentIndex + 1);

                return (
                  <div key={m.questionId} className="flex gap-4 border-b pb-4 last:border-0 last:pb-0">
                    {/* ── Question column ─────────────────────────────────── */}
                    <div className="flex-1 prose prose-sm dark:prose-invert border rounded bg-background/50 p-3 opacity-90 overflow-x-auto whitespace-normal break-words">
                      <div className="text-xs font-semibold text-muted-foreground mb-2 uppercase tracking-wider">
                        Question {qNum}
                      </div>
                      <ReactMarkdown 
                        remarkPlugins={[remarkMath]} 
                        rehypePlugins={[rehypeKatex]}
                        urlTransform={(value) => value}
                        components={{
                          img: ({ node, ...props }) => {
                            if (props.src && (props.src.match(/^[a-zA-Z]:[\\/]/) || props.src.startsWith("/"))) {
                              try {
                                const assetUrl = convertFileSrc(props.src);
                                return (
                                  <div className="relative group">
                                    <img
                                        {...props}
                                        src={assetUrl}
                                        alt={props.alt || "Diagram"}
                                        className="max-w-full rounded-md my-4"
                                        onError={(e) => {
                                          console.error("Failed to load image via asset protocol:", props.src, assetUrl);
                                          const target = e.target as HTMLImageElement;
                                          target.style.opacity = '0.5';
                                          target.title = `Failed to load: ${props.src} -> ${assetUrl}`;
                                        }}
                                      />
                                    <div className="hidden group-hover:block absolute bottom-0 left-0 bg-black/80 text-white text-[10px] p-1 truncate max-w-full">
                                      {props.src}
                                    </div>
                                  </div>
                                  );
                                } catch (e) {
                                  return <div className="text-sm text-destructive border border-destructive/20 p-2 rounded-md bg-destructive/10 text-center">Failed to convert diagram URL: {props.alt || "Image"}</div>;
                                }
                              }
                              return <img {...props} alt={props.alt || "Diagram"} className="max-w-full rounded-md my-4" />;
                            }
                          }}
                        >
                        {preprocessMath(m.rawContent)}
                      </ReactMarkdown>
                    </div>

                    {/* ── Proposed answer column ───────────────────────────── */}
                    <div className="flex-1 flex flex-col prose prose-sm dark:prose-invert border rounded bg-background p-3 relative overflow-x-auto whitespace-normal break-words">
                      <div className="flex items-center justify-between gap-2 mb-2">
                        <div className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                          Proposed Answer
                        </div>
                        <div className="flex items-center gap-2">
                          <label
                            htmlFor={`swap-${m.questionId}`}
                            className="text-xs font-medium text-muted-foreground"
                          >
                            Match to:
                          </label>
                          <select
                            id={`swap-${m.questionId}`}
                            // The selected value is the current question's own number.
                            value={qNum}
                            onChange={(e) => handleSwap(currentIndex, parseInt(e.target.value, 10))}
                            className="text-xs bg-background border border-border rounded px-2 py-1 focus:ring-1 focus:ring-primary focus:outline-none"
                          >
                            {localMappings.map((opt, optIdx) => {
                              const optQNum = extractQuestionNumber(opt.rawContent, optIdx + 1);
                              return (
                                <option key={opt.questionId} value={optQNum}>
                                  Question {optQNum}
                                </option>
                              );
                            })}
                          </select>
                        </div>
                      </div>
                      <ReactMarkdown 
                        remarkPlugins={[remarkMath]} 
                        rehypePlugins={[rehypeKatex]}
                        urlTransform={(value) => value}
                        components={{
                          img: ({ node, ...props }) => {
                            if (props.src && (props.src.match(/^[a-zA-Z]:[\\/]/) || props.src.startsWith("/"))) {
                              try {
                                const assetUrl = convertFileSrc(props.src);
                                return (
                                  <img
                                      {...props}
                                      src={assetUrl}
                                      alt={props.alt || "Diagram"}
                                      className="max-w-full rounded-md my-4"
                                      onError={() => console.error("Failed to load image via asset protocol:", props.src)}
                                    />
                                  );
                                } catch (e) {
                                  return <div className="text-sm text-destructive border border-destructive/20 p-2 rounded-md bg-destructive/10 text-center">Failed to convert diagram URL: {props.alt || "Image"}</div>;
                                }
                              }
                              return <img {...props} alt={props.alt || "Diagram"} className="max-w-full rounded-md my-4" />;
                            }
                          }}
                        >
                        {preprocessMath(m.proposedAnswer)}
                      </ReactMarkdown>
                    </div>
                  </div>
                );
              })}
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
