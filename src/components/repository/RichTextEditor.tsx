import { Component, ErrorInfo, useState, ReactNode } from 'react';
import {
  MDXEditor,
  headingsPlugin,
  listsPlugin,
  quotePlugin,
  thematicBreakPlugin,
  markdownShortcutPlugin,
  toolbarPlugin,
  UndoRedo,
  BoldItalicUnderlineToggles,
  BlockTypeSelect,
  CodeToggle,
  ListsToggle,
  CreateLink,
  linkPlugin,
  linkDialogPlugin,
  imagePlugin,
  InsertImage,
  tablePlugin,
  codeBlockPlugin,
  codeMirrorPlugin,
  usePublisher,
  insertMarkdown$
} from '@mdxeditor/editor';
import '@mdxeditor/editor/style.css';
import './RichTextEditor.css';
import { cn, sanitizeMarkdownMath } from '@/lib/utils';
import { convertFileSrc } from "@tauri-apps/api/core";

const InlineMathButton = () => {
  const insertMarkdown = usePublisher(insertMarkdown$);
  return (
    <button type="button" title="Inline LaTeX" aria-label="Inline LaTeX" onClick={() => insertMarkdown('$ $')}>
      <span className="font-serif italic font-bold text-[13px] leading-none text-white">fx</span>
    </button>
  );
};

const BlockMathButton = () => {
  const insertMarkdown = usePublisher(insertMarkdown$);
  return (
    <button type="button" title="Block LaTeX" aria-label="Block LaTeX" onClick={() => insertMarkdown('\n$$\n\n$$\n')}>
      <span className="font-serif font-bold text-[15px] leading-none text-white">∑</span>
    </button>
  );
};

interface RichTextEditorProps {
  markdown: string;
  onChange: (markdown: string) => void;
  placeholder?: string;
  className?: string;
}

interface ErrorBoundaryProps {
  children: ReactNode;
  fallback: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
}

class MDXErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(_: Error): ErrorBoundaryState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("MDXEditor crashed:", error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallback;
    }
    return this.props.children;
  }
}

export function RichTextEditor({ markdown, onChange, placeholder, className }: RichTextEditorProps) {
  const [lexicalError, setLexicalError] = useState<Error | null>(null);
  const sanitizedMarkdown = sanitizeMarkdownMath(markdown);
  
  return (
    <div className={cn("w-full border border-border rounded-md bg-transparent focus-within:ring-1 focus-within:ring-primary flex flex-col min-h-0 overflow-y-auto relative", className)}>
      <MDXErrorBoundary fallback={
        <div className="flex flex-col flex-1 h-full relative">
          <div className="p-2 bg-destructive/10 text-destructive text-[11px] font-semibold border-b border-destructive/20">
            Rich text editor failed to load due to malformed markdown (e.g. unclosed tags or math). You can edit the raw markdown below:
          </div>
          <textarea 
            className="flex-1 w-full h-full min-h-[200px] p-4 bg-transparent outline-none text-sm font-mono text-foreground resize-none"
            value={sanitizedMarkdown}
            onChange={(e) => onChange(e.target.value)}
            placeholder={placeholder}
          />
        </div>
      }>
        {lexicalError ? (
          <ThrowError error={lexicalError} />
        ) : (
          <MDXEditor
            className="dark-theme dark-editor mdxeditor-theme"
            markdown={sanitizedMarkdown}
            onChange={onChange}
            onError={(err) => {
              console.error("MDXEditor Lexical error:", err);
              setLexicalError(new Error(err.error || String(err)));
            }}
            contentEditableClassName="min-h-[200px] outline-none px-4 py-3 text-sm flex-1 font-mono text-foreground"
            placeholder={placeholder}
        plugins={[
          headingsPlugin(),
          listsPlugin(),
          quotePlugin(),
          thematicBreakPlugin(),
          linkPlugin(),
          linkDialogPlugin(),
          imagePlugin({
            imageUploadHandler: async (image: File) => {
              return new Promise((resolve, reject) => {
                const reader = new FileReader();
                reader.onload = () => resolve(reader.result as string);
                reader.onerror = reject;
                reader.readAsDataURL(image);
              });
            },
            imagePreviewHandler: async (imageSource: string) => {
              if (imageSource && (imageSource.match(/^[a-zA-Z]:[\\/]/) || imageSource.startsWith("/"))) {
                try {
                  return convertFileSrc(imageSource);
                } catch (e) {
                  return imageSource;
                }
              }
              return imageSource;
            }
          }),
          tablePlugin(),
          codeBlockPlugin({ defaultCodeBlockLanguage: 'txt' }),
          codeMirrorPlugin({ codeBlockLanguages: { js: 'JavaScript', ts: 'TypeScript', txt: 'Text', python: 'Python' } }),
          markdownShortcutPlugin(),
          toolbarPlugin({
            toolbarContents: () => (
              <div className="flex flex-wrap items-center gap-0 w-full bg-transparent mdx-sleek-toolbar">
                <UndoRedo />
                <div className="w-px h-3 bg-white/30 mx-1" />
                <BlockTypeSelect />
                <div className="w-px h-3 bg-white/30 mx-1" />
                <BoldItalicUnderlineToggles />
                <div className="w-px h-3 bg-white/30 mx-1" />
                <CodeToggle />
                <InlineMathButton />
                <BlockMathButton />
                <div className="w-px h-3 bg-white/30 mx-1" />
                <CreateLink />
                <InsertImage />
                <div className="w-px h-3 bg-white/30 mx-1" />
                <ListsToggle />
              </div>
            )
          })
        ]}
      />)}
      </MDXErrorBoundary>
    </div>
  );
}

function ThrowError({ error }: { error: Error }) {
  throw error;
}
