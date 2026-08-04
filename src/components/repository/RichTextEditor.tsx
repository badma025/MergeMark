import { useState, useRef, useEffect, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import remarkGfm from 'remark-gfm';
import 'katex/dist/katex.min.css';
import { cn } from '@/lib/utils';
import { preprocessMath } from './QuestionCard';
import {
  Bold,
  Italic,
  Code,
  List,
  ListOrdered,
  Table as TableIcon,
  Eye,
  Edit3,
  Columns,
  Undo2,
  Redo2,
  ChevronDown,
  Pi,
  Sigma
} from 'lucide-react';

interface RichTextEditorProps {
  markdown: string;
  onChange: (markdown: string) => void;
  placeholder?: string;
  className?: string;
}

const COMMON_SYMBOLS = [
  { label: '±', latex: '\\pm ' },
  { label: '×', latex: '\\times ' },
  { label: '÷', latex: '\\div ' },
  { label: '≠', latex: '\\neq ' },
  { label: '≤', latex: '\\leq ' },
  { label: '≥', latex: '\\geq ' },
  { label: '≈', latex: '\\approx ' },
  { label: '∞', latex: '\\infty ' },
  { label: 'θ', latex: '\\theta ' },
  { label: 'π', latex: '\\pi ' },
  { label: 'α', latex: '\\alpha ' },
  { label: 'β', latex: '\\beta ' },
  { label: 'Δ', latex: '\\Delta ' },
  { label: '∂', latex: '\\partial ' },
  { label: '→', latex: '\\to ' },
  { label: '∈', latex: '\\in ' },
];

export function RichTextEditor({ markdown, onChange, placeholder, className }: RichTextEditorProps) {
  const [mode, setMode] = useState<'write' | 'preview' | 'split'>('write');
  const [showSymbols, setShowSymbols] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const symbolsRef = useRef<HTMLDivElement>(null);

  // History stack for undo/redo
  const historyRef = useRef<string[]>([markdown]);
  const historyIndexRef = useRef<number>(0);

  // Close symbols dropdown on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (symbolsRef.current && !symbolsRef.current.contains(e.target as Node)) {
        setShowSymbols(false);
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const pushHistory = useCallback((newValue: string) => {
    const nextHistory = historyRef.current.slice(0, historyIndexRef.current + 1);
    if (nextHistory[nextHistory.length - 1] !== newValue) {
      nextHistory.push(newValue);
      if (nextHistory.length > 50) nextHistory.shift();
      historyRef.current = nextHistory;
      historyIndexRef.current = nextHistory.length - 1;
    }
  }, []);

  const handleUndo = () => {
    if (historyIndexRef.current > 0) {
      historyIndexRef.current -= 1;
      const prev = historyRef.current[historyIndexRef.current];
      onChange(prev);
    }
  };

  const handleRedo = () => {
    if (historyIndexRef.current < historyRef.current.length - 1) {
      historyIndexRef.current += 1;
      const next = historyRef.current[historyIndexRef.current];
      onChange(next);
    }
  };

  const insertText = (prefix: string, suffix: string = '', defaultText: string = '') => {
    const textarea = textareaRef.current;
    if (!textarea) {
      const updated = (markdown || '') + prefix + defaultText + suffix;
      onChange(updated);
      pushHistory(updated);
      return;
    }

    textarea.focus();
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const current = markdown || '';
    const selected = current.slice(start, end) || defaultText;

    const before = current.slice(0, start);
    const after = current.slice(end);

    const replacement = prefix + selected + suffix;
    const newValue = before + replacement + after;

    onChange(newValue);
    pushHistory(newValue);

    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        const cursorStart = start + prefix.length;
        const cursorEnd = cursorStart + selected.length;
        textareaRef.current.setSelectionRange(cursorStart, cursorEnd);
      }
    }, 0);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Undo: Ctrl+Z or Cmd+Z
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z' && !e.shiftKey) {
      e.preventDefault();
      handleUndo();
      return;
    }
    // Redo: Ctrl+Y or Ctrl+Shift+Z
    if ((e.ctrlKey || e.metaKey) && (e.key.toLowerCase() === 'y' || (e.shiftKey && e.key.toLowerCase() === 'z'))) {
      e.preventDefault();
      handleRedo();
      return;
    }
    // Bold: Ctrl+B
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') {
      e.preventDefault();
      insertText('**', '**', 'bold text');
      return;
    }
    // Italic: Ctrl+I
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'i') {
      e.preventDefault();
      insertText('*', '*', 'italic text');
      return;
    }
    // Math: Ctrl+M
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'm') {
      e.preventDefault();
      insertText('$', '$', 'x');
      return;
    }
    // Tab key: Insert 2 spaces
    if (e.key === 'Tab') {
      e.preventDefault();
      insertText('  ');
    }
  };

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    onChange(val);
    pushHistory(val);
  };

  return (
    <div className={cn(
      "w-full rounded-md border border-border bg-card/60 flex flex-col min-h-0 relative transition-all duration-150",
      "focus-within:border-white focus-within:ring-1 focus-within:ring-white",
      className
    )}>
      {/* ── Sleek Custom Toolbar ── */}
      <div className="flex flex-wrap items-center justify-between gap-1 p-1.5 bg-zinc-950/90 border-b border-border/80 text-zinc-300 rounded-t-md select-none">
        {/* Left: Formatting & Math Buttons */}
        <div className="flex flex-wrap items-center gap-0.5">
          {/* Undo / Redo */}
          <button
            type="button"
            onClick={handleUndo}
            title="Undo (Ctrl+Z)"
            aria-label="Undo"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors disabled:opacity-30"
          >
            <Undo2 className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={handleRedo}
            title="Redo (Ctrl+Y)"
            aria-label="Redo"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors disabled:opacity-30"
          >
            <Redo2 className="size-3.5" />
          </button>

          <div className="w-px h-3.5 bg-zinc-700 mx-1" />

          {/* Bold, Italic, Code */}
          <button
            type="button"
            onClick={() => insertText('**', '**', 'bold text')}
            title="Bold (Ctrl+B)"
            aria-label="Bold"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors font-bold"
          >
            <Bold className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={() => insertText('*', '*', 'italic text')}
            title="Italic (Ctrl+I)"
            aria-label="Italic"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors italic"
          >
            <Italic className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={() => insertText('`', '`', 'code')}
            title="Inline Code"
            aria-label="Code"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors"
          >
            <Code className="size-3.5" />
          </button>

          <div className="w-px h-3.5 bg-zinc-700 mx-1" />

          {/* ── LaTeX Math Controls ── */}
          <button
            type="button"
            onClick={() => insertText('$', '$', 'x')}
            title="Inline Math: $...$ (Ctrl+M)"
            aria-label="Inline LaTeX"
            className="px-2 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs font-serif italic font-bold text-sky-400"
          >
            fx
          </button>
          <button
            type="button"
            onClick={() => insertText('\n$$\n', '\n$$\n', 'f(x) = \\dots')}
            title="Block Math: $$...$$"
            aria-label="Block LaTeX"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors text-sky-400"
          >
            <Sigma className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={() => insertText('\\frac{', '}{b}', 'a')}
            title="Fraction: \frac{a}{b}"
            aria-label="Fraction"
            className="px-1.5 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs font-serif font-semibold"
          >
            a/b
          </button>
          <button
            type="button"
            onClick={() => insertText('\\sqrt{', '}', 'x')}
            title="Square Root: \sqrt{x}"
            aria-label="Square Root"
            className="px-1.5 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs font-serif"
          >
            √x
          </button>
          <button
            type="button"
            onClick={() => insertText('^{', '}', '2')}
            title="Superscript / Power: ^{2}"
            aria-label="Superscript"
            className="px-1.5 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs font-mono"
          >
            x²
          </button>
          <button
            type="button"
            onClick={() => insertText('_{', '}', '1')}
            title="Subscript: _{1}"
            aria-label="Subscript"
            className="px-1.5 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs font-mono"
          >
            x₁
          </button>
          <button
            type="button"
            onClick={() => insertText('\\begin{pmatrix}\n  ', ' & b \\\\\n  c & d\n\\end{pmatrix}', 'a')}
            title="Matrix: \begin{pmatrix} ... \end{pmatrix}"
            aria-label="Matrix"
            className="px-1.5 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs font-mono font-semibold"
          >
            [M]
          </button>

          {/* Math Symbols Picker Dropdown */}
          <div className="relative" ref={symbolsRef}>
            <button
              type="button"
              onClick={() => setShowSymbols(!showSymbols)}
              title="Insert Math Symbol"
              aria-label="Math Symbols"
              className={cn(
                "flex items-center gap-0.5 px-1.5 py-1 rounded hover:bg-zinc-800 hover:text-white transition-colors text-xs",
                showSymbols && "bg-zinc-800 text-white"
              )}
            >
              <Pi className="size-3.5 text-amber-400" />
              <ChevronDown className="size-2.5 opacity-70" />
            </button>

            {showSymbols && (
              <div className="absolute top-full left-0 mt-1 z-50 p-2 bg-zinc-900 border border-zinc-700 rounded-lg shadow-xl grid grid-cols-4 gap-1 min-w-[140px] animate-in fade-in zoom-in-95 duration-100">
                {COMMON_SYMBOLS.map((s, idx) => (
                  <button
                    key={idx}
                    type="button"
                    onClick={() => {
                      insertText(s.latex);
                      setShowSymbols(false);
                    }}
                    className="flex items-center justify-center p-1.5 rounded hover:bg-zinc-800 text-zinc-200 hover:text-white text-sm font-serif"
                    title={s.latex}
                  >
                    {s.label}
                  </button>
                ))}
              </div>
            )}
          </div>

          <div className="w-px h-3.5 bg-zinc-700 mx-1" />

          {/* Lists & Tables */}
          <button
            type="button"
            onClick={() => insertText('\n- ', '', 'List item')}
            title="Bulleted List"
            aria-label="Bullet List"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors"
          >
            <List className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={() => insertText('\n1. ', '', 'Numbered item')}
            title="Numbered List"
            aria-label="Numbered List"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors"
          >
            <ListOrdered className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={() => insertText('\n| Option | Description |\n|---|---|\n| **A** | ', ' |\n| **B** |  |\n', 'Value')}
            title="Markdown Table"
            aria-label="Table"
            className="p-1.5 rounded hover:bg-zinc-800 hover:text-white transition-colors"
          >
            <TableIcon className="size-3.5" />
          </button>
        </div>

        {/* Right: View Mode Toggle Tabs */}
        <div className="flex items-center bg-zinc-900 border border-zinc-800 rounded p-0.5 text-xs">
          <button
            type="button"
            onClick={() => setMode('write')}
            className={cn(
              "flex items-center gap-1 px-2 py-0.5 rounded transition-colors text-[11px]",
              mode === 'write' ? "bg-zinc-800 text-white font-medium shadow-xs" : "text-zinc-400 hover:text-zinc-200"
            )}
            title="Write Mode"
          >
            <Edit3 className="size-3" />
            Write
          </button>
          <button
            type="button"
            onClick={() => setMode('split')}
            className={cn(
              "flex items-center gap-1 px-2 py-0.5 rounded transition-colors text-[11px]",
              mode === 'split' ? "bg-zinc-800 text-white font-medium shadow-xs" : "text-zinc-400 hover:text-zinc-200"
            )}
            title="Split Mode (Live Preview)"
          >
            <Columns className="size-3" />
            Split
          </button>
          <button
            type="button"
            onClick={() => setMode('preview')}
            className={cn(
              "flex items-center gap-1 px-2 py-0.5 rounded transition-colors text-[11px]",
              mode === 'preview' ? "bg-zinc-800 text-white font-medium shadow-xs" : "text-zinc-400 hover:text-zinc-200"
            )}
            title="Preview Mode"
          >
            <Eye className="size-3" />
            Preview
          </button>
        </div>
      </div>

      {/* ── Editor / Preview Content ── */}
      <div className="flex-1 flex min-h-[220px] overflow-hidden">
        {/* Write Panel */}
        {(mode === 'write' || mode === 'split') && (
          <div className={cn(
            "flex-1 flex flex-col min-h-0 bg-transparent",
            mode === 'split' && "border-r border-border/80"
          )}>
            <textarea
              ref={textareaRef}
              value={markdown || ''}
              onChange={handleChange}
              onKeyDown={handleKeyDown}
              placeholder={placeholder || "Type markdown and LaTeX equations here..."}
              className="w-full flex-1 p-3.5 bg-transparent text-sm font-mono text-foreground leading-relaxed outline-none resize-none overflow-y-auto selection:bg-primary/30"
              spellCheck={false}
            />
          </div>
        )}

        {/* Live Preview Panel */}
        {(mode === 'preview' || mode === 'split') && (
          <div className="flex-1 flex flex-col min-h-0 bg-card/40 p-4 overflow-y-auto">
            {markdown?.trim() ? (
              <div className="text-sm leading-relaxed text-foreground prose prose-sm dark:prose-invert max-w-none prose-p:my-1 prose-pre:my-1 break-words">
                <ReactMarkdown
                  remarkPlugins={[remarkMath, remarkGfm]}
                  rehypePlugins={[[rehypeKatex, { throwOnError: false, strict: false }]]}
                  urlTransform={(value) => value}
                >
                  {preprocessMath(markdown)}
                </ReactMarkdown>
              </div>
            ) : (
              <div className="flex items-center justify-center h-full text-xs text-muted-foreground italic">
                Nothing to preview. Type markdown or math in the editor.
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
