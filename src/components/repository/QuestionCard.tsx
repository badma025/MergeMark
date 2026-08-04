import { useState, useRef } from "react";
import "katex/dist/katex.min.css";
import { RichTextEditor } from "./RichTextEditor";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Plus, Trash2, Pencil, AlertTriangle, ShieldCheck } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import { cn, preprocessMathString } from "@/lib/utils";
import { remarkMathFix } from "@/lib/remark-math-fix";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useTaxonomy } from "@/lib/TaxonomyContext";
import { toast } from "sonner";
import Zoom from "react-medium-image-zoom";
import "react-medium-image-zoom/dist/styles.css";

/**
 * Phase 0: Diagram renderer with click-to-enlarge.
 * Diagrams are now rendered at ~200 DPI from the PDF pipeline, but the
 * card CSS caps them at the content width (`max-w-full`). Clicking opens
 * the image at native resolution in a modal so users can inspect axis
 * labels, circuit symbols, or fine geometry that gets squeezed in a card.
 */
function DiagramImg({
  src,
  alt,
}: {
  src: string;
  alt: string;
}) {
  const isLocal = /^[a-zA-Z]:[\\/]/.test(src) || src.startsWith("/");
  const resolved = isLocal ? convertFileSrc(src) : src;
  return (
    <div className="relative group/diag my-4">
      <Zoom classDialog="custom-zoom-dark" zoomMargin={40}>
        <img
          src={resolved}
          alt={alt}
          loading="lazy"
          decoding="async"
          className="max-w-full rounded-md cursor-zoom-in ring-1 ring-border/60 hover:ring-primary/40 transition-shadow"
          onError={(e) => {
            console.error("Failed to load diagram:", src, resolved);
            const target = e.target as HTMLImageElement;
            target.style.opacity = "0.5";
            target.title = `Failed to load: ${src}`;
          }}
        />
      </Zoom>
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
  onAddToWorksheet?: (id: string) => void;
  onDelete?: (id: string) => void;
  onUpdate?: (id: string, newContent: string, newMarks: number, newAnswerContent?: string, newTopics?: string[], newModule?: string) => void;
}

export function QuestionCard(props: QuestionCardProps) {
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
  let parsedTopics: string[] = [];
  try {
    if (topics) {
      parsedTopics = JSON.parse(topics);
      if (!Array.isArray(parsedTopics)) parsedTopics = [];
    }
  } catch (e) {
    console.error("Failed to parse topics:", e);
  }

  let displayContent = stripAnswerSpaces(content ?? "");
  const strippedAnswerContent = stripAnswerSpaces(answerContent ?? "");

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
          <Badge className="bg-primary/15 text-primary hover:bg-primary/20 border-primary/20 text-xs font-semibold shrink-0">
            {marks} {marks === 1 ? "mark" : "marks"}
          </Badge>
        </div>

        {/* Right: Action buttons (never squished, distinct hover styling) */}
        <div className="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity duration-150 -mr-1 -mt-1">
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

      {/* ── Question / Answer Content (Crossfade) ── */}
      <div className="relative text-sm leading-relaxed text-foreground prose prose-sm dark:prose-invert max-w-none prose-p:my-1 prose-pre:my-1 break-words">
        
        {/* Question Content */}
        <div 
          className={cn(
            "transition-opacity duration-200 ease-in-out overflow-x-auto",
            isShowingAnswer ? "opacity-0 absolute inset-0 pointer-events-none" : "opacity-100 relative"
          )}
        >
          <ReactMarkdown 
            remarkPlugins={[remarkMath, remarkGfm, remarkMathFix]} 
            rehypePlugins={[[rehypeKatex, { throwOnError: false, strict: false }]]}
            urlTransform={(value) => value}
            components={{
              img: ({ node, ...props }) => {
                if (!props.src) return null;
                return (
                  <DiagramImg
                    src={props.src}
                    alt={props.alt || "Diagram"}
                  />
                );
              },
            }}
          >
            {preprocessMathString(displayContent)}
          </ReactMarkdown>
        </div>

        {/* Answer Content */}
        <div 
          className={cn(
            "transition-opacity duration-200 ease-in-out overflow-x-auto",
            isShowingAnswer ? "opacity-100 relative" : "opacity-0 absolute inset-0 pointer-events-none"
          )}
        >
          <div className="font-semibold text-xs text-muted-foreground mb-2 uppercase tracking-wider">Mark Scheme Answer</div>
          <ReactMarkdown 
            remarkPlugins={[remarkMath, remarkGfm, remarkMathFix]} 
            rehypePlugins={[[rehypeKatex, { throwOnError: false, strict: false }]]}
            urlTransform={(value) => value}
            components={{
              img: ({ node, ...props }) => {
                if (!props.src) return null;
                return (
                  <DiagramImg
                    src={props.src}
                    alt={props.alt || "Diagram"}
                  />
                );
              },
            }}
          >
            {preprocessMathString(answerContent ?? "")}
          </ReactMarkdown>
        </div>
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

      {/* ── Footer: Add to Worksheet & Show Answer ── */}
      <div className="flex items-center justify-between pt-1">
        <div>
          {answerContent && answerContent.trim() !== "" && (
            <Button
              variant="secondary"
              size="sm"
              className="text-xs h-7 px-3 transition-colors"
              onClick={(e) => {
                e.stopPropagation();
                setIsShowingAnswer(!isShowingAnswer);
              }}
            >
              {isShowingAnswer ? "Show Question" : "Show Answer"}
            </Button>
          )}
        </div>
        <Button
          id={`add-to-worksheet-${id}`}
          size="sm"
          className={cn(
            "gap-1.5 text-xs font-semibold",
            "bg-primary text-primary-foreground",
            "opacity-0 translate-y-1 transition-all duration-200",
            "group-hover:opacity-100 group-hover:translate-y-0"
          )}
          onClick={(e) => {
            e.stopPropagation();
            onAddToWorksheet?.(id);
          }}
          aria-label={`Add question ${id} to worksheet`}
        >
          <Plus className="size-3.5" />
          Add to Worksheet
        </Button>
      </div>
    </article>
  );
}