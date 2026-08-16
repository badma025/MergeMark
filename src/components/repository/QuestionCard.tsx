import { useState, useRef, memo } from "react";
import "katex/dist/katex.min.css";
import { RichTextEditor } from "./RichTextEditor";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Plus, Trash2, Pencil, AlertTriangle, ShieldCheck, Maximize2, ZoomIn, ZoomOut, Download, Copy, Check, X, CheckCircle2, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useTaxonomy } from "@/lib/TaxonomyContext";
import { toast } from "sonner";
import { ExamMarkdownRenderer } from "@/components/common/ExamMarkdownRenderer";
import { preprocessExamMarkdown } from "@/lib/preprocess-math";

/**
 * Interactive Diagram Lightbox Modal with zoom, copy, and download controls.
 */
function DiagramLightboxModal({
  isOpen,
  onClose,
  src,
  alt,
}: {
  isOpen: boolean;
  onClose: () => void;
  src: string;
  alt: string;
}) {
  const [scale, setScale] = useState(1);
  const isLocal = /^[a-zA-Z]:[\\/]/.test(src) || src.startsWith("/");
  const resolved = isLocal ? convertFileSrc(src) : src;

  const handleZoomIn = () => setScale((s) => Math.min(s + 0.25, 3.5));
  const handleZoomOut = () => setScale((s) => Math.max(s - 0.25, 0.5));
  const handleReset = () => setScale(1);

  const handleDownload = async () => {
    try {
      const a = document.createElement("a");
      a.href = resolved;
      a.download = alt ? `${alt.replace(/\s+/g, "_")}.png` : "diagram.png";
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      toast.success("Diagram downloaded");
    } catch {
      toast.error("Failed to download diagram");
    }
  };

  const handleCopy = async () => {
    try {
      const res = await fetch(resolved);
      const blob = await res.blob();
      await navigator.clipboard.write([
        new ClipboardItem({ [blob.type || "image/png"]: blob }),
      ]);
      toast.success("Diagram copied to clipboard");
    } catch {
      toast.error("Failed to copy image to clipboard");
    }
  };

  if (!isOpen) return null;

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-5xl w-[95vw] max-h-[90vh] p-0 overflow-hidden bg-background/95 backdrop-blur-md border-border/80 shadow-2xl flex flex-col">
        <DialogHeader className="px-6 py-4 border-b border-border/60 flex flex-row items-center justify-between space-y-0">
          <div>
            <DialogTitle className="text-base font-semibold">{alt || "Diagram Inspection"}</DialogTitle>
            <p className="text-xs text-muted-foreground mt-0.5">High-Resolution Vector/Pixel Crop · 200 DPI</p>
          </div>
          <div className="flex items-center gap-1.5 mr-6">
            <Button variant="outline" size="icon" className="size-8" onClick={handleZoomOut} title="Zoom Out (-)">
              <ZoomOut className="size-4" />
            </Button>
            <Button variant="outline" size="sm" className="h-8 px-2.5 text-xs font-mono" onClick={handleReset} title="Reset Zoom">
              {Math.round(scale * 100)}%
            </Button>
            <Button variant="outline" size="icon" className="size-8" onClick={handleZoomIn} title="Zoom In (+)">
              <ZoomIn className="size-4" />
            </Button>
            <div className="w-px h-4 bg-border mx-1" />
            <Button variant="outline" size="sm" className="h-8 gap-1.5 text-xs" onClick={handleCopy} title="Copy Image">
              <Copy className="size-3.5" />
              <span>Copy</span>
            </Button>
            <Button variant="outline" size="sm" className="h-8 gap-1.5 text-xs" onClick={handleDownload} title="Download PNG">
              <Download className="size-3.5" />
              <span>Download</span>
            </Button>
          </div>
        </DialogHeader>
        <div className="flex-1 min-h-0 overflow-auto p-6 flex items-center justify-center bg-black/40">
          <img
            src={resolved}
            alt={alt}
            style={{ transform: `scale(${scale})`, transition: "transform 0.15s ease-out" }}
            className="max-w-full max-h-[70vh] object-contain rounded-md shadow-lg ring-1 ring-border/40"
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Diagram renderer with click-to-enlarge lightbox modal.
 */
function DiagramImg({
  src,
  alt,
}: {
  src: string;
  alt: string;
}) {
  const [isLightboxOpen, setIsLightboxOpen] = useState(false);
  const isLocal = /^[a-zA-Z]:[\\/]/.test(src) || src.startsWith("/");
  const resolved = isLocal ? convertFileSrc(src) : src;

  return (
    <div className="relative group/diag my-4 inline-block max-w-full">
      <div 
        onClick={() => setIsLightboxOpen(true)}
        className="relative cursor-zoom-in group/img overflow-hidden rounded-lg border border-border/70 hover:border-primary/50 transition-all shadow-sm hover:shadow-md bg-card/50"
      >
        <img
          src={resolved}
          alt={alt}
          loading="lazy"
          decoding="async"
          className="max-w-full rounded-lg transition-transform duration-200 group-hover/img:scale-[1.01]"
          onError={(e) => {
            console.error("Failed to load diagram:", src, resolved);
            const target = e.target as HTMLImageElement;
            target.style.opacity = "0.5";
            target.title = `Failed to load: ${src}`;
          }}
        />
        <div className="absolute inset-0 bg-black/0 group-hover/img:bg-black/25 transition-colors flex items-end justify-end p-2 opacity-0 group-hover/img:opacity-100 pointer-events-none">
          <span className="bg-background/90 backdrop-blur-sm text-foreground text-[11px] font-medium px-2 py-1 rounded-md shadow-sm border border-border/60 flex items-center gap-1">
            <Maximize2 className="size-3 text-primary" />
            Click to Enlarge
          </span>
        </div>
      </div>

      <DiagramLightboxModal
        isOpen={isLightboxOpen}
        onClose={() => setIsLightboxOpen(false)}
        src={src}
        alt={alt}
      />
    </div>
  );
}

/**
 * Strip Answer Spaces (dots, underscores, coordinate tuples)
 */
export function stripAnswerSpaces(raw: string): string {
  if (!raw) return "";
  let s = raw;

  // 1. Coordinate Answer Spaces e.g. (..........,..........) or (_____,_____)
  // Matches coordinates with 2 or more components, where each component is an answer line of 4+ dots/underscores.
  s = s.replace(/\(\s*(?:(?:[.][ \t]*){4,}|(?:[_][ \t]*){4,})(?:\s*,\s*(?:(?:[.][ \t]*){4,}|(?:[_][ \t]*){4,}))+\s*\)/g, "");

  // 2. Main Answer Spaces
  // Matches 8 or more dots or underscores, optionally separated by spaces.
  // Also captures preceding currency symbols (£, $, €) and commonly attached
  // proceeding units (cm^2, %, mm, etc.) without partially stripping words.
  s = s.replace(/(?:[£$€]\s*)?(?:(?:[.][ \t]*){8,}|(?:[_][ \t]*){8,})(?:\s*(?:cm\^?[23]?|mm\^?[23]?|m\^?[23]?|km|g|grams?|kg|kilograms?|mg|l|litres?|ml|seconds?|secs?|s|mins?|minutes?|hours?|hrs?|p|pence|%|°|degrees?|m\/s|km\/h|m\/s\^?2)(?![a-zA-Z]))?/gi, "");

  return s;
}

export interface QuestionCardProps {
  id: string;
  subject: string;
  subtopic: string;
  topics?: string;
  marks: number;
  content: string;
  mathSnippet: string;
  /** Whether the snippet is a code block (true) or math formula (false) */
  isCode?: boolean;
  answerContent?: string;
  className?: string;
  module?: string;
  /** Set by the pipeline when extraction anomalies occurred — shows REVIEW badge */
  needsReview?: boolean;
  /** Set when a mark scheme is re-imported over an existing answer — shows STALE ANSWER banner */
  answerStale?: boolean;
  /** Whether this question is already selected in the active worksheet */
  isAdded?: boolean;
  onAddToWorksheet?: (id: string) => void;
  onDelete?: (id: string) => void;
  onUpdate?: (id: string, newContent: string, newMarks: number, newAnswerContent?: string, newTopics?: string[], newModule?: string) => void;
}

export const QuestionCard = memo(function QuestionCard(props: QuestionCardProps) {
  const {
    id,
    subject,
    module,
    marks,
    content,
    topics,
    answerContent,
    className,
    needsReview: initialNeedsReview,
    answerStale: initialAnswerStale,
    isAdded = false,
    onAddToWorksheet,
    onUpdate,
    onDelete,
  } = props;
  const { subjects, topicsBySubject } = useTaxonomy();
  const displaySubject = subjects.find(s => s.id === subject)?.name || subject;
  const [isEditing, setIsEditing] = useState(false);
  const [isShowingAnswer, setIsShowingAnswer] = useState(false);
  const [needsReview, setNeedsReview] = useState(!!initialNeedsReview);
  const [answerStale, setAnswerStale] = useState(!!initialAnswerStale);
  const [isAddedBtnHovered, setIsAddedBtnHovered] = useState(false);
  let parsedTopics: string[] = [];
  try {
    if (topics) {
      parsedTopics = JSON.parse(topics);
      if (!Array.isArray(parsedTopics)) parsedTopics = [];
    }
  } catch (e) {
    console.error("Failed to parse topics:", e);
  }

  let displayContent = preprocessExamMarkdown(stripAnswerSpaces(content ?? ""));
  const strippedAnswerContent = preprocessExamMarkdown(stripAnswerSpaces(answerContent ?? ""));

  // Task 4: math_snippet logic removed — content is the single source of truth.
  // The DB migration sets math_snippet = '' for all existing rows.

  const [editContent, setEditContent] = useState(displayContent);
  const [editMarks, setEditMarks] = useState(marks);
  const [editAnswerContent, setEditAnswerContent] = useState(strippedAnswerContent);
  const [editTopics, setEditTopics] = useState<string[]>(parsedTopics);

  const lastCloseTime = useRef(0);

  function handleSave(e?: React.MouseEvent) {
    e?.stopPropagation();
    onUpdate?.(id, editContent, editMarks, editAnswerContent || undefined, editTopics, module);
    lastCloseTime.current = Date.now();
    setIsEditing(false);
  }

  function handleCancel(e?: React.MouseEvent) {
    e?.stopPropagation();
    setEditContent(displayContent);
    setEditMarks(marks);
    setEditAnswerContent(strippedAnswerContent);
    setEditTopics(parsedTopics);
    lastCloseTime.current = Date.now();
    setIsEditing(false);
  }

  return (
    <article
      onClick={() => {
        if (isEditing || Date.now() - lastCloseTime.current < 300) return;
        setEditContent(displayContent);
        setEditMarks(marks);
        setEditAnswerContent(strippedAnswerContent);
        setEditTopics(parsedTopics);
        setIsEditing(true);
      }}
      className={cn(
        "group relative flex flex-col gap-3 rounded-xl border border-border bg-card p-4 shadow-sm",
        "transition-all duration-200 hover:border-primary/40 hover:shadow-md hover:shadow-primary/5 cursor-pointer",
        className
      )}
    >
      {/* ── Header: Badges & Action Buttons ── */}
      <div className="flex items-start justify-between gap-3 min-w-0">
        {/* Left: Badges (wrap naturally without overlapping actions) */}
        <div className="flex flex-wrap items-center gap-1.5 min-w-0 flex-1">
          {needsReview && (
            <Badge className="text-xs font-semibold tracking-wide bg-amber-500/15 text-amber-700 dark:text-amber-400 border-amber-500/30 hover:bg-amber-500/25 cursor-help shrink-0" title="Extracted with vision fallback or low confidence">
              <AlertTriangle className="size-3 mr-1 inline" />
              REVIEW
            </Badge>
          )}
          <Badge
            className="text-xs font-medium tracking-wide bg-zinc-800 text-zinc-50 hover:bg-zinc-800/90 dark:bg-zinc-200 dark:text-zinc-900 dark:hover:bg-zinc-200/90 shrink-0"
          >
            {displaySubject}
          </Badge>
          {module && module !== "General" && module !== "Unknown" && (
            <Badge
              className="text-xs font-medium tracking-wide bg-purple-900/50 text-purple-200 border-purple-800 hover:bg-purple-900/60 shrink-0"
            >
              {module}
            </Badge>
          )}
          {parsedTopics.map((topic, i) => (
            <Badge
              key={i}
              variant="outline"
              className="text-xs font-medium bg-blue-900/50 text-blue-200 border-blue-800 shrink-0"
            >
              {topic}
            </Badge>
          ))}
          {(() => {
            const diffMatch = displayContent.match(/(?:\*{0,2}\[Difficulty:\s*([^*\]\)]+)\*{0,2}\]|\(([★*]{1,5}\+?)\))/i);
            if (diffMatch) {
              const rating = (diffMatch[1] || diffMatch[2]).trim();
              return (
                <Badge className="bg-amber-500/15 text-amber-300 border-amber-500/30 text-xs font-semibold shrink-0 gap-1">
                  <span>Difficulty:</span>
                  <span className="font-mono text-amber-200">{rating}</span>
                </Badge>
              );
            }
            if (marks != null && marks > 0) {
              return (
                <Badge className="bg-primary/15 text-primary hover:bg-primary/20 border-primary/20 text-xs font-semibold shrink-0">
                  {marks} {marks === 1 ? "mark" : "marks"}
                </Badge>
              );
            }
            return null;
          })()}
        </div>

        {/* Right: Action buttons (never squished, distinct hover styling) */}
        <div className="flex items-center gap-1 shrink-0 opacity-70 group-hover:opacity-100 focus-within:opacity-100 transition-opacity duration-150 -mr-1 -mt-1">
          {!isEditing && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                navigator.clipboard.writeText(displayContent);
                toast.success("Question copied to clipboard");
              }}
              title="Copy Question Markdown"
              aria-label={`Copy question ${id}`}
              className={cn(
                "flex items-center justify-center rounded-md p-1.5",
                "text-muted-foreground transition-all duration-150",
                "hover:bg-primary/10 hover:text-primary",
                "focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/60"
              )}
            >
              <Copy className="size-4" />
            </button>
          )}
          {!isEditing && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setEditContent(displayContent);
                setEditMarks(marks);
                setEditAnswerContent(strippedAnswerContent);
                setEditTopics(parsedTopics);
                setIsEditing(true);
              }}
              title="Edit Question"
              aria-label={`Edit question ${id}`}
              className={cn(
                "flex items-center justify-center rounded-md p-1.5",
                "text-muted-foreground transition-all duration-150",
                "hover:bg-primary/10 hover:text-primary",
                "focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/60"
              )}
            >
              <Pencil className="size-4" />
            </button>
          )}
          {!isEditing && needsReview && (
            <button
              type="button"
              onClick={async (e) => {
                e.stopPropagation();
                try {
                  await invoke("mark_question_verified", { id });
                  setNeedsReview(false);
                  setAnswerStale(false);
                  toast.success("Question marked as verified");
                } catch (e: any) {
                  toast.error(e.toString());
                }
              }}
              aria-label={`Mark question ${id} as verified`}
              className={cn(
                "flex items-center justify-center rounded-md p-1.5",
                "text-emerald-600 transition-all duration-150",
                "hover:bg-emerald-500/10 hover:text-emerald-500",
                "focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-500/60"
              )}
              title="Mark as Verified"
            >
              <ShieldCheck className="size-4" />
            </button>
          )}
          <button
            id={`delete-question-${id}`}
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDelete?.(id);
            }}
            title="Delete Question"
            aria-label={`Delete question ${id}`}
            className={cn(
              "flex items-center justify-center rounded-md p-1.5",
              "text-muted-foreground transition-all duration-150",
              "hover:bg-destructive/10 hover:text-destructive",
              "focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-destructive/60"
            )}
          >
            <Trash2 className="size-4" />
          </button>
        </div>
      </div>

      {/* ── Stale Answer Warning Banner ── */}
      {answerStale && (
        <div className="flex items-center gap-2 bg-amber-500/10 border border-amber-500/20 text-amber-700 dark:text-amber-400 p-2.5 rounded-lg text-sm -mx-1 mt-0 mb-1">
          <AlertTriangle className="size-4 shrink-0" />
          <div className="flex-1">
            <strong>Stale Answer:</strong> This mark scheme answer may be out of date compared to the recently updated question content.
          </div>
        </div>
      )}

      {/* ── Question Content ── */}
      <div className="relative text-sm leading-relaxed text-foreground min-w-0">
        <div className="overflow-x-auto min-w-0 max-w-full">
          <ExamMarkdownRenderer
            content={displayContent}
            imageRenderer={(src, alt) => (
              <DiagramImg src={src} alt={alt || "Diagram"} />
            )}
          />
        </div>

        {/* ── Mark Scheme Accordion Drawer ── */}
        {strippedAnswerContent && strippedAnswerContent.trim() !== "" && (
          <div className="mt-3 pt-2 border-t border-border/50 not-prose">
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setIsShowingAnswer(!isShowingAnswer);
              }}
              className="flex w-full items-center justify-between py-1.5 px-3 rounded-lg bg-muted/40 hover:bg-muted/70 border border-border/60 cursor-pointer transition-colors text-xs font-semibold text-muted-foreground hover:text-foreground select-none"
            >
              <div className="flex items-center gap-1.5">
                <CheckCircle2 className="size-3.5 text-emerald-500 shrink-0" />
                <span>Mark Scheme Solution</span>
              </div>
              <ChevronDown className={cn("size-3.5 transition-transform duration-200", isShowingAnswer && "rotate-180")} />
            </button>

            {isShowingAnswer && (
              <div className="mt-2 p-3 rounded-lg bg-muted/20 border border-border/60 text-sm animate-in fade-in-50 slide-in-from-top-1 duration-150">
                <ExamMarkdownRenderer
                  content={strippedAnswerContent}
                  imageRenderer={(src, alt) => (
                    <DiagramImg src={src} alt={alt || "Diagram"} />
                  )}
                />
              </div>
            )}
          </div>
        )}
      </div>

      {/* ── Edit Modal ── */}
      <Dialog open={isEditing} onOpenChange={(open) => { if (!open) handleCancel(); }}>
        <DialogContent className="max-w-[95vw] sm:max-w-[95vw] h-[95vh] w-full flex flex-col p-6">
          <DialogHeader>
            <DialogTitle>Edit Question</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-4 py-2 flex-1 min-h-0 overflow-y-auto pr-2 pb-6">
            {/* Top Controls Row */}
            <div className="flex items-center gap-4 flex-wrap bg-muted/30 p-3 rounded-lg border border-border/50 shrink-0">
              <div className="flex items-center gap-2">
                <label className="text-sm font-semibold text-foreground whitespace-nowrap">Marks:</label>
                <input
                  type="number"
                  min={1}
                  max={100}
                  value={editMarks}
                  onChange={(e) => setEditMarks(parseInt(e.target.value) || 1)}
                  className="w-20 p-1.5 text-sm bg-background border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-primary/50"
                />
              </div>
            </div>

            {/* Topics selection */}
            <div className="flex flex-col gap-2 shrink-0">
              <label className="text-sm font-semibold text-foreground">Topics:</label>
              <div className="flex flex-wrap items-center gap-1.5">
                {(() => {
                  if (displaySubject === "All") return [];
                  const subjectMods = topicsBySubject[displaySubject] || {};
                  return Object.values(subjectMods).flat();
                })().map((topic) => {
                  const isSelected = editTopics.includes(topic);
                  return (
                    <Badge
                      key={topic}
                      variant={isSelected ? "default" : "outline"}
                      className={cn(
                        "cursor-pointer transition-colors text-xs font-medium py-0.5",
                        isSelected ? "bg-blue-600 hover:bg-blue-700 text-white border-blue-600" : "hover:bg-accent border-border"
                      )}
                      onClick={(e) => {
                        e.stopPropagation();
                        setEditTopics(prev =>
                          prev.includes(topic)
                            ? prev.filter(t => t !== topic)
                            : [...prev, topic]
                        );
                      }}
                    >
                      {topic}
                    </Badge>
                  );
                })}
              </div>
            </div>

            {/* Content Editors */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 flex-1 min-h-[320px] p-0.5">
              <div className="flex flex-col gap-1.5 flex-1 min-h-0">
                <div className="min-h-[44px] flex flex-col justify-end">
                  <label className="text-sm font-semibold text-foreground">Question Content:</label>
                  <p className="text-xs text-muted-foreground">Markdown supported. Inline math: $...$, Block math: $$...$$</p>
                </div>
                <RichTextEditor 
                  markdown={editContent}
                  onChange={setEditContent}
                  className="flex-1 w-full min-h-[300px]"
                />
              </div>
              <div className="flex flex-col gap-1.5 flex-1 min-h-0">
                <div className="min-h-[44px] flex flex-col justify-end">
                  <label className="text-sm font-semibold text-foreground">Mark Scheme Answer (Optional):</label>
                  <p className="text-xs text-muted-foreground">Markdown supported. Optional mark scheme or model answer.</p>
                </div>
                <RichTextEditor 
                  markdown={editAnswerContent}
                  onChange={setEditAnswerContent}
                  placeholder="Paste or edit the mark scheme answer here..."
                  className="flex-1 w-full min-h-[300px]"
                />
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-2 mt-auto pt-4 border-t border-border shrink-0">
            <Button variant="outline" onClick={handleCancel}>Cancel</Button>
            <Button onClick={handleSave}>Save Changes</Button>
          </div>
        </DialogContent>
      </Dialog>

      {/* ── Footer: Add to Worksheet Button ── */}
      <div className="flex items-center justify-end pt-1">
        {isAdded ? (
          <Button
            id={`add-to-worksheet-${id}`}
            size="sm"
            variant="secondary"
            onMouseEnter={() => setIsAddedBtnHovered(true)}
            onMouseLeave={() => setIsAddedBtnHovered(false)}
            className={cn(
              "group/addedbtn gap-1.5 text-xs font-semibold select-none cursor-pointer",
              "transition-all duration-150 shadow-xs",
              "hover:scale-[1.03] active:scale-[0.97]",
              isAddedBtnHovered
                ? "bg-destructive/15 text-destructive border border-destructive/40 shadow-destructive/15 hover:bg-destructive/25 hover:border-destructive/60 hover:ring-2 hover:ring-destructive/30"
                : "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/30 hover:bg-emerald-500/25 hover:border-emerald-500/50 hover:ring-2 hover:ring-emerald-500/20"
            )}
            onClick={(e) => {
              e.stopPropagation();
              setIsAddedBtnHovered(false);
              onAddToWorksheet?.(id);
            }}
            aria-label={`Remove question ${id} from worksheet`}
          >
            {isAddedBtnHovered ? (
              <>
                <X className="size-3.5 transition-transform duration-150 group-hover/addedbtn:rotate-90" />
                <span>Remove</span>
              </>
            ) : (
              <>
                <Check className="size-3.5 transition-transform duration-150 group-hover/addedbtn:scale-110" />
                <span>Added</span>
              </>
            )}
          </Button>
        ) : (
          <Button
            id={`add-to-worksheet-${id}`}
            size="sm"
            className={cn(
              "group/addbtn gap-1.5 text-xs font-semibold cursor-pointer select-none",
              "bg-primary text-primary-foreground shadow-xs",
              "hover:bg-primary/85 dark:hover:bg-primary/90 hover:brightness-110",
              "hover:ring-2 hover:ring-primary/40 hover:shadow-md hover:shadow-primary/20",
              "hover:scale-[1.04] active:scale-[0.96]",
              "transition-all duration-150"
            )}
            onClick={(e) => {
              e.stopPropagation();
              onAddToWorksheet?.(id);
            }}
            aria-label={`Add question ${id} to worksheet`}
          >
            <Plus className="size-3.5 transition-transform duration-150 group-hover/addbtn:rotate-90 group-hover/addbtn:scale-110" />
            <span>Add to Worksheet</span>
          </Button>
        )}
      </div>
    </article>
  );
});