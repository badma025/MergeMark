import React, { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import remarkGfm from 'remark-gfm';
import 'katex/dist/katex.min.css';
import { cn } from '@/lib/utils';
import { preprocessExamMarkdown } from '@/lib/preprocess-math';
import { remarkMathFix } from '@/lib/remark-math-fix';
import type { Root, Paragraph } from 'mdast';
import { visit } from 'unist-util-visit';
import { ErrorBoundary } from '@/components/common/ErrorBoundary';

export interface ExamMarkdownRendererProps {
  content: string;
  className?: string;
  imageRenderer?: (src: string, alt?: string) => React.ReactNode;
}

// Regex to capture trailing mark allocations like: [4 marks], (3 marks), [Total: 5 marks], [1 mark], [4]
const MARK_ALLOCATION_REGEX = /(\s*(?:\[|\()\s*(?:Total:?\s*)?(\d+\s*marks?|\d+)\s*(?:\]|\))\s*)$/i;

/**
 * Checks whether a given URL points to an image file or asset protocol
 */
function isImageUrl(url?: string): boolean {
  if (!url || typeof url !== 'string') return false;
  const trimmed = url.trim();
  return (
    /\.(?:png|jpe?g|webp|svg|gif|bmp|tiff?)(?:\?.*)?$/i.test(trimmed) ||
    trimmed.startsWith('data:image/') ||
    trimmed.startsWith('appdata://') ||
    trimmed.startsWith('asset://')
  );
}

/**
 * Remark plugin to preserve single newlines in question/exam paragraphs as hard line breaks (<br />).
 * Essential for multi-line physics reactions, nuclear decay data, and given constants.
 */
function remarkPreserveBreaks() {
  return (tree: Root) => {
    visit(tree, 'paragraph', (node: Paragraph) => {
      const newChildren: any[] = [];
      for (const child of node.children) {
        if (child.type === 'text' && typeof child.value === 'string' && child.value.includes('\n')) {
          const lines = child.value.split('\n');
          lines.forEach((line, idx) => {
            if (idx > 0) {
              newChildren.push({ type: 'break' });
            }
            if (line.length > 0) {
              newChildren.push({ type: 'text', value: line });
            }
          });
        } else {
          newChildren.push(child);
        }
      }
      node.children = newChildren;
    });
  };
}

/**
 * Inspects children of a paragraph to extract trailing mark allocations
 * and push them flush-right against the right margin.
 */
function ParagraphWithFlushMarks({
  node,
  children,
  ...props
}: React.HTMLAttributes<HTMLParagraphElement> & { node?: unknown }) {
  const childrenArray = React.Children.toArray(children);
  if (childrenArray.length === 0) return <p {...props}>{children}</p>;

  const lastChild = childrenArray[childrenArray.length - 1];

  // If the last child is a string and contains mark allocation at the end
  if (typeof lastChild === 'string') {
    const match = lastChild.match(MARK_ALLOCATION_REGEX);
    if (match) {
      const markString = match[1].trim(); // e.g. "[4 marks]"
      const cleanString = lastChild.slice(0, match.index);
      const leadingChildren = childrenArray.slice(0, -1);

      return (
        <p className="my-1.5 leading-relaxed text-foreground relative clearfix after:content-[''] after:block after:clear-both" {...props}>
          {leadingChildren}
          {cleanString}
          {/* Flush-right mark allocation badge */}
          <span
            className={cn(
              "float-right ml-3 my-0.5 inline-flex items-center gap-1 shrink-0",
              "font-mono font-bold text-[11px] tracking-tight text-foreground/80 dark:text-foreground/90",
              "bg-muted/80 dark:bg-muted/40 border border-border/80 px-2 py-0.5 rounded-md",
              "shadow-xs select-none tabular-nums print:border-black/30 print:bg-transparent"
            )}
            title="Mark Allocation"
          >
            {markString.startsWith('[') || markString.startsWith('(') ? markString : `[${markString}]`}
          </span>
        </p>
      );
    }
  }

  return <p className="my-1.5 leading-relaxed text-foreground" {...props}>{children}</p>;
}

/**
 * Recursively inspects children (strings, arrays, <p> elements) to extract [MCQ:X] tag.
 * Allows loose lists (separated by blank lines) to seamlessly unpack and render as cards.
 */
function extractMcqInfo(children: React.ReactNode): { mcqKey: string | null; cleanContent: React.ReactNode } {
  if (!children) return { mcqKey: null, cleanContent: children };

  if (typeof children === 'string') {
    const trimmed = children.trim();
    if (!trimmed) return { mcqKey: null, cleanContent: children };
    const match = children.match(/^\s*\[MCQ:([A-E])\]\s*([\s\S]*)$/i);
    if (match) {
      return { mcqKey: match[1].toUpperCase(), cleanContent: match[2] };
    }
    return { mcqKey: null, cleanContent: children };
  }

  if (Array.isArray(children)) {
    if (children.length === 0) return { mcqKey: null, cleanContent: children };
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      if (typeof child === 'string' && child.trim() === '') {
        continue;
      }
      const extracted = extractMcqInfo(child);
      if (extracted.mcqKey) {
        const leading = children.slice(0, i);
        const rest = children.slice(i + 1);
        const cleanContent = [
          ...leading,
          extracted.cleanContent,
          ...rest,
        ].filter(c => typeof c !== 'string' || c.trim() !== '');
        return { mcqKey: extracted.mcqKey, cleanContent };
      }
      break;
    }
    return { mcqKey: null, cleanContent: children };
  }

  if (React.isValidElement(children)) {
    const props = children.props as { children?: React.ReactNode; [key: string]: any };
    if (props && props.children !== undefined) {
      const extracted = extractMcqInfo(props.children);
      if (extracted.mcqKey) {
        return { mcqKey: extracted.mcqKey, cleanContent: extracted.cleanContent };
      }
    }
  }

  return { mcqKey: null, cleanContent: children };
}

/**
 * Recursively detects whether rendered MCQ content contains an image/diagram.
 */
function containsImage(node: React.ReactNode): boolean {
  return React.Children.toArray(node).some((child) => {
    if (!React.isValidElement(child)) return false;
    const props = child.props as { src?: unknown; href?: unknown; children?: React.ReactNode };
    if (child.type === 'img' || typeof props.src === 'string') return true;
    if (typeof props.href === 'string' && isImageUrl(props.href)) return true;
    return containsImage(props.children);
  });
}

/**
 * Custom <ul> and <ol> list renderer with MCQ Grid transformation.
 */
function ListRenderer({ node, children, ...props }: any) {
  const childrenArray = React.Children.toArray(children);
  // Check if any child is an MCQ option
  const isMcqList = childrenArray.some((child: any) => {
    if (child?.props?.className?.includes('mcq-item')) return true;
    const { mcqKey } = extractMcqInfo(child?.props?.children);
    return mcqKey !== null;
  });

  if (isMcqList) {
    // Sort MCQ children alphabetically by key (A -> B -> C -> D)
    const sortedChildren = [...childrenArray].sort((a: any, b: any) => {
      const keyA = extractMcqInfo(a?.props?.children).mcqKey || '';
      const keyB = extractMcqInfo(b?.props?.children).mcqKey || '';
      return keyA.localeCompare(keyB);
    });

    return (
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5 my-3 not-prose">
        {sortedChildren}
      </div>
    );
  }

  return (
    <ul className="my-2 ml-5 list-disc space-y-1 text-sm text-foreground marker:text-muted-foreground" {...props}>
      {children}
    </ul>
  );
}

/**
 * Custom <li> list item renderer handling MCQ cards vs standard bullets.
 */
function ListItemRenderer({ node, children, ...props }: any) {
  const { mcqKey, cleanContent } = extractMcqInfo(children);

  if (mcqKey) {
    const hasDiagram = containsImage(cleanContent);
    return (
      <div
        className={cn(
          "mcq-item group/mcq flex items-start gap-3 p-3 rounded-lg",
          "border border-border/80 bg-card/60 hover:bg-accent/40 hover:border-primary/40",
          "transition-all duration-150 shadow-xs cursor-default select-text min-w-0 relative",
          hasDiagram && "flex-col items-center justify-center p-3.5"
        )}
      >
        <span
          className={cn(
            "flex items-center justify-center size-6 rounded-md shrink-0 font-bold text-xs font-mono",
            "bg-primary/10 text-primary border border-primary/20",
            "group-hover/mcq:bg-primary group-hover/mcq:text-primary-foreground transition-colors",
            hasDiagram && "absolute top-2.5 left-2.5 z-10"
          )}
        >
          {mcqKey}
        </span>
        <div
          className={cn(
            "flex-1 min-w-0 text-sm text-foreground leading-snug pt-0.5 break-words",
            hasDiagram && "flex flex-col items-center justify-center gap-2 pt-4 pb-1 text-center [&_p]:my-0 w-full overflow-hidden",
            hasDiagram && "mcq-option-media"
          )}
        >
          {cleanContent}
        </div>
      </div>
    );
  }

  return (
    <li className="leading-relaxed" {...props}>
      {children}
    </li>
  );
}

/**
 * Production Exam Markdown Renderer Component
 * Supports KaTeX math ($ / $$), GFM tables, Task Lists, Flush-Right Marks, Image Links, and MCQ Cards.
 */
export const ExamMarkdownRenderer = memo(function ExamMarkdownRenderer({
  content,
  className,
  imageRenderer,
}: ExamMarkdownRendererProps) {
  let processedContent = "";
  try {
    processedContent = preprocessExamMarkdown(content || "");
  } catch (err) {
    console.error("[ExamMarkdownRenderer] Preprocessing failed:", err);
    processedContent = content || "";
  }

  return (
    <ErrorBoundary fallbackTitle="Could not render formatting">
      <div
        className={cn(
          "prose prose-sm dark:prose-invert max-w-none",
          "prose-p:my-1.5 prose-headings:font-bold prose-headings:tracking-tight",
          "break-words [overflow-wrap:anywhere] min-w-0",
          className
        )}
      >
        <ReactMarkdown
          remarkPlugins={[remarkMath, remarkGfm, remarkPreserveBreaks, remarkMathFix]}
          // throwOnError:false + errorColor: a failed math block renders as a
          // short styled token instead of crashing the render pass or
          // swallowing the text / MCQ grid below it.
          rehypePlugins={[[rehypeKatex, { throwOnError: false, strict: false, errorColor: '#dc2626' }]]}
          urlTransform={(value) => value}
          components={{
            p: ParagraphWithFlushMarks,
            ul: ListRenderer,
            li: ListItemRenderer,

            // ── Links & Image Links ───────────────────────────────────────
            a: ({ node, href, children, ...aProps }) => {
              if (href && isImageUrl(href)) {
                const altText =
                  typeof children === 'string'
                    ? children
                    : Array.isArray(children) && typeof children[0] === 'string'
                    ? children[0]
                    : "Diagram";
                if (imageRenderer) {
                  return <>{imageRenderer(href, altText)}</>;
                }
                return (
                  <img
                    src={href}
                    alt={altText}
                    className="max-w-full rounded-md my-2 border border-border"
                  />
                );
              }
              return (
                <a
                  href={href}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-primary underline underline-offset-2 hover:text-primary/80"
                  {...aProps}
                >
                  {children}
                </a>
              );
            },

            // ── GFM Table Overrides ───────────────────────────────────────
            table: ({ node, ...tableProps }) => (
              <div className="overflow-x-auto my-3.5 max-w-full rounded-lg border border-border/80 bg-card/40 shadow-xs not-prose">
                <table className="w-full text-sm text-left border-collapse" {...tableProps} />
              </div>
            ),
            thead: ({ node, ...theadProps }) => (
              <thead className="bg-muted/60 dark:bg-muted/40 border-b border-border/80 text-foreground" {...theadProps} />
            ),
            tbody: ({ node, ...tbodyProps }) => (
              <tbody className="divide-y divide-border/50 text-foreground" {...tbodyProps} />
            ),
            tr: ({ node, ...trProps }) => (
              <tr className="hover:bg-muted/25 transition-colors" {...trProps} />
            ),
            th: ({ node, ...thProps }) => (
              <th
                className="p-2.5 px-3.5 font-semibold text-xs text-foreground/90 uppercase tracking-wider border-r border-border/40 last:border-r-0 text-left align-middle"
                {...thProps}
              />
            ),
            td: ({ node, ...tdProps }) => (
              <td
                className="p-2.5 px-3.5 text-sm text-foreground border-r border-border/30 last:border-r-0 align-middle leading-snug"
                {...tdProps}
              />
            ),

            // ── GFM Task Lists & Checkboxes ──────────────────────────────
            input: ({ node, ...inputProps }) => {
              if (inputProps.type === 'checkbox') {
                return (
                  <input
                    {...inputProps}
                    disabled
                    className="rounded border-border text-primary focus:ring-primary size-3.5 mr-1.5 align-middle cursor-default"
                  />
                );
              }
              return <input {...inputProps} />;
            },

            // ── GFM Strikethrough ────────────────────────────────────────
            del: ({ node, ...delProps }) => (
              <del className="line-through text-muted-foreground opacity-75" {...delProps} />
            ),

            // ── Diagram / Image Handling ──────────────────────────────────
            img: ({ node, ...imgProps }) => {
              if (imageRenderer && imgProps.src) {
                return <>{imageRenderer(imgProps.src, imgProps.alt)}</>;
              }
              return <img {...imgProps} alt={imgProps.alt || "Diagram"} className="max-w-full rounded-md my-3 border border-border" />;
            },
          }}
        >
          {processedContent}
        </ReactMarkdown>
      </div>
    </ErrorBoundary>
  );
});

