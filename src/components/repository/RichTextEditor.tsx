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
import { cn } from '@/lib/utils';
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

export function RichTextEditor({ markdown, onChange, placeholder, className }: RichTextEditorProps) {
  return (
    <div className={cn("w-full border border-border rounded-md bg-transparent focus-within:ring-1 focus-within:ring-primary flex flex-col min-h-0 overflow-y-auto relative", className)}>
      <MDXEditor
        className="dark-theme dark-editor mdxeditor-theme"
        markdown={markdown}
        onChange={onChange}
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
      />
    </div>
  );
}
