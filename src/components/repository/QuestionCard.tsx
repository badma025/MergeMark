import { useState } from "react";
import "katex/dist/katex.min.css";
import { RichTextEditor } from "./RichTextEditor";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Plus, Trash2, Pencil } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useTaxonomy } from "@/lib/TaxonomyContext";

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

export function preprocessMath(raw: string, isCode?: boolean, subject?: string): string {
  if (!raw) return "";

  let s = raw.trim();

  // ── Strip Answer Spaces (Dots and Underscores) ──────────────────────────
  s = stripAnswerSpaces(s);


  // ── 0: Convert Markdown Code Blocks to LaTeX Math Blocks ───────────────
  // If the AI outputs ```latex ... ```, ReactMarkdown treats it as a `<pre>` block,
  // preventing Katex from rendering it. We swap them for `$$` blocks.
  if (!isCode && subject !== "Computer Science") {
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

  // ── 0.5: Convert Markdown Tables to LaTeX Arrays ─────────────────────────
  // The system should render tables automatically in latex (KaTeX arrays)
  s = s.replace(/(?:^|\n)((?:[ \t]*\|[^\n]+\|[ \t]*(?:\n|$))+)/g, (match, tableBlock) => {
    const lines = tableBlock.trim().split('\n');
    if (lines.length < 3) return match; 
    
    const divider = lines[1];
    if (!/^[ \t]*\|(?:[ \t]*:?-+:?[ \t]*\|)+[ \t]*$/.test(divider)) return match;

    const cols = divider.split('|').slice(1, -1).map((c: string) => c.trim());
    const format = cols.map((c: string) => {
      if (c.startsWith(':') && c.endsWith(':')) return 'c';
      if (c.endsWith(':')) return 'r';
      return 'l';
    }).join('|');

    let latex = `\n\n$$\n\\begin{array}{|${format}|}\n\\hline\n`;

    for (let i = 0; i < lines.length; i++) {
      if (i === 1) continue; // skip divider
      
      const line = lines[i];
      const cells = line.split('|').slice(1, -1).map((c: string) => c.trim());
      
      const latexCells = cells.map((cell: string) => {
        const parts = cell.split(/(\$[^$]+\$)/g);
        return parts.map((part: string) => {
          if (part.startsWith('$') && part.endsWith('$')) {
            return part.slice(1, -1);
          } else if (part !== '') {
            // Escape & and % which break LaTeX arrays, wrap in \text{}
            let text = part.replace(/&/g, '\\&').replace(/%/g, '\\%').replace(/\$/g, '\\$');
            // KaTeX \text{} preserves spaces but we must ensure it doesn't break
            return `\\text{${text}}`;
          }
          return '';
        }).join(' ');
      });

      latex += latexCells.join(' & ') + ' \\\\\n\\hline\n';
    }

    latex += `\\end{array}\n$$\n\n`;
    return (match.startsWith('\n') ? '\n' : '') + latex;
  });

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
  module?: string;
  onAddToWorksheet?: (id: string) => void;
  onDelete?: (id: string) => void;
  onUpdate?: (id: string, newContent: string, newMarks: number, newAnswerContent?: string, newTopics?: string[], newModule?: string) => void;
}

export function QuestionCard({
  id,
  subject,
  module,
  marks,
  content,
  isCode,
  mathSnippet,
  topics,
  answerContent,
  className,
  onAddToWorksheet,
  onUpdate,
  onDelete,
}: QuestionCardProps) {
  const { topicsBySubject } = useTaxonomy();
  const [isEditing, setIsEditing] = useState(false);
  const [isShowingAnswer, setIsShowingAnswer] = useState(false);
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
  
  const snippet = (mathSnippet || "").trim();
  if (snippet !== "") {
    const contentTrim = displayContent.trimEnd();
    if (contentTrim.endsWith(snippet)) {
      displayContent = contentTrim.substring(0, contentTrim.length - snippet.length).trimEnd();
    }
    if (isCode) {
      displayContent += `\n\n\`\`\`\n${snippet}\n\`\`\``;
    } else {
      if (snippet.startsWith("$$") && snippet.endsWith("$$")) {
        displayContent += `\n\n${snippet}`;
      } else if (snippet.startsWith("$") && snippet.endsWith("$") && !snippet.includes("\n")) {
        displayContent += `\n\n${snippet}`;
      } else {
        displayContent += `\n\n$$\n${snippet}\n$$`;
      }
    }
  }

  const [editContent, setEditContent] = useState(displayContent);
  const [editMarks, setEditMarks] = useState(marks);
  const [editAnswerContent, setEditAnswerContent] = useState(strippedAnswerContent);
  const [editTopics, setEditTopics] = useState<string[]>(parsedTopics);

  function handleSave(e?: React.MouseEvent) {
    e?.stopPropagation();
    onUpdate?.(id, editContent, editMarks, editAnswerContent || undefined, editTopics, module);
    setIsEditing(false);
  }

  function handleCancel(e?: React.MouseEvent) {
    e?.stopPropagation();
    setEditContent(displayContent);
    setEditMarks(marks);
    setEditAnswerContent(strippedAnswerContent);
    setEditTopics(parsedTopics);
    setIsEditing(false);
  }

  return (
    <article
      onClick={() => {
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
      {/* ── Action buttons — top-right corner, visible on hover ── */}
      <div className="absolute top-3 right-3 flex gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-all duration-150 z-10">
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
        {module && module !== "General" && module !== "Unknown" && (
          <Badge
            className="text-xs font-medium tracking-wide bg-purple-900/50 text-purple-200 border-purple-800 hover:bg-purple-900/60"
          >
            {module}
          </Badge>
        )}
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
      <div className="relative text-sm leading-relaxed text-foreground prose prose-sm dark:prose-invert max-w-none prose-p:my-1 prose-pre:my-1 break-words">
        
        {/* Question Content */}
        <div 
          className={cn(
            "transition-opacity duration-200 ease-in-out overflow-x-auto",
            isShowingAnswer ? "opacity-0 absolute inset-0 pointer-events-none" : "opacity-100 relative"
          )}
        >
          <ReactMarkdown 
            remarkPlugins={[remarkMath, remarkGfm]} 
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
              {preprocessMath(displayContent, isCode, subject)}
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
            remarkPlugins={[remarkMath, remarkGfm]} 
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
              {preprocessMath(answerContent ?? "", isCode, subject)}
          </ReactMarkdown>
        </div>
      </div>
      {/* ── Edit Modal ── */}
      <Dialog open={isEditing} onOpenChange={(open) => { if (!open) handleCancel(); }}>
        <DialogContent className="max-w-[95vw] sm:max-w-[95vw] h-[95vh] w-full flex flex-col p-6">
          <DialogHeader>
            <DialogTitle>Edit Question</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-4 py-2 flex-1 min-h-0 overflow-y-auto pr-2">
            {/* Top Controls Row */}
            <div className="flex items-center gap-4 flex-wrap bg-muted/30 p-3 rounded-lg border border-border/50">
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
            <div className="flex flex-col gap-2">
              <label className="text-sm font-semibold text-foreground">Topics:</label>
              <div className="flex flex-wrap items-center gap-1.5">
                {(() => {
                  if (subject === "All") return [];
                  const subjectMods = topicsBySubject[subject] || {};
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
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 flex-1 min-h-[300px]">
              <div className="flex flex-col gap-2 h-full">
                <div>
                  <label className="text-sm font-semibold text-foreground">Question Content:</label>
                  <p className="text-xs text-muted-foreground">Markdown supported. Inline math: $...$, Block math: $$...$$</p>
                </div>
                <RichTextEditor 
                  markdown={editContent}
                  onChange={setEditContent}
                  className="flex-1 w-full h-full"
                />
              </div>
              <div className="flex flex-col gap-2 h-full">
                <label className="text-sm font-semibold text-foreground">Mark Scheme Answer (Optional):</label>
                <RichTextEditor 
                  markdown={editAnswerContent}
                  onChange={setEditAnswerContent}
                  placeholder="Paste or edit the mark scheme answer here..."
                  className="flex-1 w-full h-full mt-[18px]"
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
