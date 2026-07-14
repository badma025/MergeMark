import { useState } from "react";
import "katex/dist/katex.min.css";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Plus, Trash2, Pencil } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { cn } from "@/lib/utils";
import { convertFileSrc } from "@tauri-apps/api/core";

/**
 * Regex that matches display-worthy LaTeX operators.
 * \\{1,2} handles both \int (1 backslash) and \\int (AI double-escaping in JSON).
 * (?![a-z]) prevents matching longer names like \integer but allows \int_2, \int^n etc.
 */
const DISPLAY_OP_RE =
  /\\{1,2}(?:int|iint|iiint|oint|sum|prod|coprod|lim|bigcup|bigcap|bigsqcup|bigvee|bigwedge)(?![a-z])/;

/** Convert \\cmd → \cmd (AI sometimes double-escapes backslashes from JSON) */
function fixSlashes(s: string): string {
  return s.replace(/\\{2}([a-zA-Z])/g, "\\$1");
}

/**
 * Normalise content so every display-worthy equation is wrapped in $$...$$
 * before ReactMarkdown + rehype-katex see it.
 *
 * Uses global multiline regex passes (not line-by-line) so patterns that span
 * multiple lines are handled correctly in a single replacement call.
 *
 * Patterns handled (all AI inconsistencies we have observed):
 *   A) $$ expr $$           — already correct, left alone
 *   B) $              ← AI using $ on its own line as a "block" delimiter
 *      \int expr
 *      $
 *   C) $\int expr$          — single-line inline wrapping a display expr
 *   D) \int expr            — raw LaTeX with no delimiters at all
 *   E) text $\int expr$ text — display expr embedded mid-sentence
 */
export function preprocessMath(raw: string, isCode?: boolean): string {
  if (!raw) return "";

  let s = raw.trim();

  // ── 0: Convert Markdown Code Blocks to LaTeX Math Blocks ───────────────
  // If the AI outputs ```latex ... ```, ReactMarkdown treats it as a `<pre>` block,
  // preventing Katex from rendering it. We swap them for `$$` blocks.
  if (!isCode) {
    // Match ```latex, ```math, ```tex, or just empty ``` if it's not a code question
    // We also match any stray `$` signs OUTSIDE the backticks so they get absorbed and removed!
    s = s.replace(/\$*\s*```(?:latex|math|tex)?\s*\n([\s\S]*?)\n```\s*\$*/gi, (_m, inner) => {
      // Sometimes the AI accidentally includes `$$` inside the code block. Strip them.
      const clean = inner.replace(/^\s*\$+\s*/, '').replace(/\s*\$+\s*$/, '').trim();
      return `\n\n$$${clean}$$\n\n`;
    });
    
    // Replace inline single backticks, absorbing stray `$` signs outside them
    // E.g. `\ln|u| + C`$$ -> $$ \ln|u| + C $$
    s = s.replace(/\$*\s*`([^`]+)`\s*\$*/g, (_m, inner) => {
      const clean = inner.replace(/^\s*\$+\s*/, '').replace(/\s*\$+\s*$/, '').trim();
      return `$$${clean}$$`;
    });
  }

  // ── A: protect already-correct $$ blocks from further processing ──────────
  // We mark them with a placeholder, restore at end.
  const blocks: string[] = [];
  
  // Helper to format block math with aligned environment if it has multiple lines
  const formatBlock = (inner: string) => {
    let clean = fixSlashes(inner.trim());
    // If it has multiple lines and isn't already using an environment, wrap in aligned
    if (clean.includes("\n") && !clean.includes("\\begin{")) {
      // Replace newlines with \\ so KaTeX renders them on separate lines
      clean = `\\begin{aligned}\n${clean.replace(/\n/g, " \\\\\n")}\n\\end{aligned}`;
    }
    const idx = blocks.length;
    blocks.push(`\n\n$$\n${clean}\n$$\n\n`);
    return `\x00BLOCK${idx}\x00`;
  };

  // First, fix AI generating unmatched delimiters: $ ... $$ or $$ ... $
  s = s.replace(/(?:^|\n)[ \t]*\$[ \t]*\n([\s\S]*?)\n[ \t]*\$\$[ \t]*(?=\n|$)/gm, (_m, inner) => `\n\n$$${inner}$$\n\n`);
  s = s.replace(/(?:^|\n)[ \t]*\$\$[ \t]*\n([\s\S]*?)\n[ \t]*\$[ \t]*(?=\n|$)/gm, (_m, inner) => `\n\n$$${inner}$$\n\n`);

  s = s.replace(/\$\$([\s\S]*?)\$\$/g, (_m, inner) => formatBlock(inner));

  // ── B: $ on its own line used as block delimiter ──────────────────────────
  s = s.replace(
    /(?:^|\n)[ \t]*\$[ \t]*\n([\s\S]*?)\n[ \t]*\$[ \t]*(?=\n|$)/gm,
    (_m, inner) => `\n\n${formatBlock(inner)}\n\n`
  );

  // ── C+E: inline $...$ (single-line) containing a display operator ─────────
  s = s.replace(/\$([^$\n]+)\$/g, (match, expr) => {
    if (DISPLAY_OP_RE.test(expr)) {
      return `\n\n${formatBlock(expr)}\n\n`;
    }
    return match;
  });

  // ── D: raw LaTeX line (no $ at all) with a display operator ──────────────
  s = s.replace(
    /^(?!\s*\x00BLOCK)([^\n$]*(?:\\{1,2}(?:int|iint|iiint|oint|sum|prod|coprod|lim|bigcup|bigcap|bigsqcup|bigvee|bigwedge)(?![a-z]))[^\n$]*)$/gm,
    (_m, line) => `\n\n${formatBlock(line)}\n\n`
  );

  // ── Restore protected $$ blocks ───────────────────────────────────────────
  s = s.replace(/\x00BLOCK(\d+)\x00/g, (_m, idx) => blocks[Number(idx)]);

  // Collapse 3+ blank lines to 2
  return s.replace(/\n{3,}/g, "\n\n").trim();
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
  onAddToWorksheet?: (id: string) => void;
  onDelete?: (id: string) => void;
  onUpdate?: (id: string, newContent: string, newMarks: number, newAnswerContent?: string) => void;
}

export function QuestionCard({
  id,
  subject,
  subtopic,
  topics,
  marks,
  content,
  answerContent,
  isCode,
  className,
  onAddToWorksheet,
  onDelete,
  onUpdate,
}: QuestionCardProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [isShowingAnswer, setIsShowingAnswer] = useState(false);
  const [editContent, setEditContent] = useState(content);
  const [editMarks, setEditMarks] = useState(marks);
  const [editAnswerContent, setEditAnswerContent] = useState(answerContent || "");

  let parsedTopics: string[] = [];
  try {
    if (topics) {
      parsedTopics = JSON.parse(topics);
      if (!Array.isArray(parsedTopics)) parsedTopics = [];
    }
  } catch (e) {
    console.error("Failed to parse topics:", e);
  }

  function handleSave() {
    onUpdate?.(id, editContent, editMarks, editAnswerContent || undefined);
    setIsEditing(false);
  }

  function handleCancel() {
    setEditContent(content);
    setEditMarks(marks);
    setEditAnswerContent(answerContent || "");
    setIsEditing(false);
  }
  return (
    <article
      onClick={() => setIsEditing(true)}
      className={cn(
        "group relative flex flex-col gap-3 rounded-xl border border-border bg-card p-4 shadow-sm",
        "transition-all duration-200 hover:border-primary/40 hover:shadow-md hover:shadow-primary/5 cursor-pointer",
        className
      )}
    >
      {/* ── Action buttons — top-right corner, visible on hover ── */}
      <div className="absolute top-3 right-3 flex gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-all duration-150 z-10">
        {!isEditing && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              setIsEditing(true);
            }}
            aria-label={`Edit question ${id}`}
            className={cn(
              "flex items-center justify-center rounded-md p-1.5",
              "text-muted-foreground/40 transition-all duration-150",
              "hover:bg-primary/10 hover:text-primary hover:opacity-100",
              "focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/60"
            )}
          >
            <Pencil className="size-3.5" />
          </button>
        )}
        <button
          id={`delete-question-${id}`}
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onDelete?.(id);
          }}
          aria-label={`Delete question ${id}`}
          className={cn(
            "flex items-center justify-center rounded-md p-1.5",
            "text-muted-foreground/40 transition-all duration-150",
            "hover:bg-destructive/10 hover:text-destructive hover:opacity-100",
            "focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-destructive/60"
          )}
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>

      {/* ── Badge row ── */}
      <div className="flex flex-wrap items-center gap-2 pr-7">
        <Badge
          className="text-xs font-medium tracking-wide bg-zinc-800 text-zinc-50 hover:bg-zinc-800/90 dark:bg-zinc-200 dark:text-zinc-900 dark:hover:bg-zinc-200/90"
        >
          {subject}
        </Badge>
        {parsedTopics.map((topic, i) => (
          <Badge
            key={i}
            variant="outline"
            className="text-xs font-medium bg-blue-900/50 text-blue-200 border-blue-800"
          >
            {topic}
          </Badge>
        ))}
        <Badge className="ml-auto bg-primary/15 text-primary hover:bg-primary/20 border-primary/20 text-xs font-semibold">
          {marks} {marks === 1 ? "mark" : "marks"}
        </Badge>
      </div>

      {/* ── Question content ── */}
      {/* ── Question / Answer Content (Crossfade) ── */}
      <div className="relative text-sm leading-relaxed text-foreground prose prose-sm dark:prose-invert max-w-none prose-p:my-1 prose-pre:my-1">
        
        {/* Question Content */}
        <div 
          className={cn(
            "transition-opacity duration-200 ease-in-out overflow-x-auto",
            isShowingAnswer ? "opacity-0 absolute inset-0 pointer-events-none" : "opacity-100 relative"
          )}
        >
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
              {preprocessMath(content ?? "", isCode)}
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
              {preprocessMath(answerContent ?? "", isCode)}
          </ReactMarkdown>
        </div>
      </div>
      {/* ── Edit Modal ── */}
      <Dialog open={isEditing} onOpenChange={setIsEditing}>
        <DialogContent className="max-w-3xl w-full max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Question</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-4 py-4">
            <div className="flex items-center gap-4">
              <label className="text-sm font-semibold text-foreground whitespace-nowrap">Marks:</label>
              <input
                type="number"
                min={1}
                max={100}
                value={editMarks}
                onChange={(e) => setEditMarks(parseInt(e.target.value) || 1)}
                className="w-24 p-2 text-sm bg-background border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-primary/50"
              />
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="flex flex-col gap-2">
                <label className="text-sm font-semibold text-foreground">Question Content (Markdown):</label>
                <textarea 
                  value={editContent}
                  onChange={(e) => setEditContent(e.target.value)}
                  className="w-full h-[400px] p-3 text-sm bg-background border border-input rounded-md font-mono resize-none focus:outline-none focus:ring-2 focus:ring-primary/50"
                />
              </div>
              <div className="flex flex-col gap-2">
                <label className="text-sm font-semibold text-foreground">Answer Content (Mark Scheme):</label>
                <textarea 
                  value={editAnswerContent}
                  onChange={(e) => setEditAnswerContent(e.target.value)}
                  placeholder="Paste or edit the mark scheme answer here..."
                  className="w-full h-[400px] p-3 text-sm bg-background border border-input rounded-md font-mono resize-none focus:outline-none focus:ring-2 focus:ring-primary/50"
                />
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={handleCancel}>Cancel</Button>
            <Button onClick={handleSave}>Save Changes</Button>
          </DialogFooter>
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
