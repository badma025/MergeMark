import React, { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import remarkGfm from 'remark-gfm';
import 'katex/dist/katex.min.css';
import { cn } from '@/lib/utils';
import { preprocessExamMarkdown } from '@/lib/preprocess-math';
import { remarkMathFix } from '@/lib/remark-math-fix';

export interface ExamMarkdownRendererProps {
  content: string;
  className?: string;
  imageRenderer?: (src: string, alt?: string) => React.ReactNode;
}

// Regex to capture trailing mark allocations like: [4 marks], (3 marks), [Total: 5 marks], [1 mark], [4]
const MARK_ALLOCATION_REGEX = /(\s*(?:\[|\()\s*(?:Total:?\s*)?(\d+\s*marks?|\d+)\s*(?:\]|\))\s*)$/i;

/**
 * Inspects children of a paragraph to extract trailing mark allocations
 * and push them flush-right against the right margin.
 */
function ParagraphWithFlushMarks({ children, ...props }: React.HTMLAttributes<HTMLParagraphElement>) {
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
 * Custom <ul> and <ol> list renderer with MCQ Grid transformation.
 */
function ListRenderer({ node, children, ...props }: any) {
  // Check if any child is an MCQ option
  const isMcqList = React.Children.toArray(children).some((child: any) => {
    return (
      child?.props?.className?.includes('mcq-item') ||
      (typeof child?.props?.children === 'string' && child.props.children.includes('[MCQ:')) ||
      (Array.isArray(child?.props?.children) &&
        typeof child.props.children[0] === 'string' &&
        child.props.children[0].includes('[MCQ:'))
    );
  });

  if (isMcqList) {
    return (
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5 my-3 not-prose">
        {children}
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
function ListItemRenderer({ children, ...props }: any) {
  let mcqKey: string | null = null;
  let mcqContent: React.ReactNode = children;

  const childrenArray = React.Children.toArray(children);
  const firstChild = childrenArray[0];

  if (typeof firstChild === 'string') {
    const mcqMatch = firstChild.match(/^\s*\[MCQ:([A-E])\]\s*(.*)$/i);
    if (mcqMatch) {
      mcqKey = mcqMatch[1].toUpperCase();
      const remainingText = mcqMatch[2];
      mcqContent = remainingText ? [remainingText, ...childrenArray.slice(1)] : childrenArray.slice(1);
    }
  }

  if (mcqKey) {
    return (
      <div
        className={cn(
          "mcq-item group/mcq flex items-start gap-3 p-3 rounded-lg",
          "border border-border/80 bg-card/60 hover:bg-accent/40 hover:border-primary/40",
          "transition-all duration-150 shadow-xs cursor-default select-text min-w-0"
        )}
      >
        <span
          className={cn(
            "flex items-center justify-center size-6 rounded-md shrink-0 font-bold text-xs font-mono",
            "bg-primary/10 text-primary border border-primary/20",
            "group-hover/mcq:bg-primary group-hover/mcq:text-primary-foreground transition-colors"
          )}
        >
          {mcqKey}
        </span>
        <div className="flex-1 min-w-0 text-sm text-foreground leading-snug pt-0.5 break-words">
          {mcqContent}
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

import { ErrorBoundary } from '@/components/common/ErrorBoundary';

/**
 * Production Exam Markdown Renderer Component
 * Supports KaTeX math ($ / $$), GFM tables, Task Lists, Flush-Right Marks, and MCQ Cards.
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
          remarkPlugins={[remarkMath, remarkGfm, remarkMathFix]}
          rehypePlugins={[[rehypeKatex, { throwOnError: false, strict: false }]]}
          urlTransform={(value) => value}
          components={{
            p: ParagraphWithFlushMarks,
            ul: ListRenderer,
            li: ListItemRenderer,

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

