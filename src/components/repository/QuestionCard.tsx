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
import { convertFileSrc } from "@tauri-apps/api/core";
import { useTaxonomy } from "@/lib/TaxonomyContext";
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
  const [isShowingAnswer] = useState(false);
  const [needsReview] = useState(!!initialNeedsReview);
  const [answerStale] = useState(!!initialAnswerStale);
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
            <Badge variant="outline" className="bg-warning/10 text-warning border-warning/30 text-xs gap-1">
              <AlertTriangle className="size-3" />
              REVIEW
            </Badge>
          )}
          {answerStale && (
            <Badge variant="outline" className="bg-destructive/10 text-destructive border-destructive/30 text-xs gap-1">
              <ShieldCheck className="size-3" />
              STALE ANSWER
            </Badge>
          )}
        </div>

        {/* Right: Action Buttons */}
        <div className="flex items-center gap-1.5 shrink-0">
          {onAddToWorksheet && (
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => { e.stopPropagation(); onAddToWorksheet?.(id); }}
              aria-label="Add to worksheet"
              className="h-7 w-7 p-0"
            >
              <Plus className="size-4" />
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => { e.stopPropagation(); setIsEditing(true); }}
            aria-label="Edit question"
            className="h-7 w-7 p-0"
          >
            <Pencil className="size-4" />
          </Button>
          {onDelete && (
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => { e.stopPropagation(); onDelete?.(id); }}
              aria-label="Delete question"
              className="h-7 w-7 p-0 text-destructive hover:bg-destructive/10"
            >
              <Trash2 className="size-4" />
            </Button>
          )}
        </div>
      </div>

      {/* ── Question & Answer ── */}
      <div className="relative">
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

            {/* Question Content Editor */}
            <div className="flex flex-col gap-2 flex-1 min-h-0">
              <label className="text-sm font-semibold text-foreground">Question:</label>
              <RichTextEditor
                markdown={editContent}
                onChange={setEditContent}
                placeholder="Enter the question text with LaTeX math..."
                className="flex-1 min-h-[200px]"
              />
            </div>

            {/* Answer Content Editor */}
            <div className="flex flex-col gap-2 shrink-0">
              <label className="text-sm font-semibold text-foreground">Mark Scheme Answer:</label>
              <RichTextEditor
                markdown={editAnswerContent}
                onChange={setEditAnswerContent}
                placeholder="Enter the mark scheme answer with LaTeX math..."
                className="flex-1 min-h-[120px]"
              />
            </div>

            {/* Dialog Actions */}
            <div className="flex justify-end gap-2 shrink-0 pt-2 border-t">
              <Button variant="outline" onClick={handleCancel}>Cancel</Button>
              <Button onClick={handleSave}>Save Changes</Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </article>
  );
}