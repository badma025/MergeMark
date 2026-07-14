import { useState, useCallback, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { ReviewSyncModal, type ProposedMapping } from "./ReviewSyncModal";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { UploadCloud, FileText, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";

import * as pdfjsLib from "pdfjs-dist";
import pdfWorker from "pdfjs-dist/build/pdf.worker.mjs?url";

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorker;

// ── IngestionDropzone ─────────────────────────────────────────────────────────

interface IngestionDropzoneProps {
  onSuccess?: () => void;
}

export function IngestionDropzone({ onSuccess }: IngestionDropzoneProps) {
  const [importMode, setImportMode] = useState<"questions" | "mark_scheme">("questions");
  const [subject, setSubject] = useState("Mathematics");
  const [pendingMappings, setPendingMappings] = useState<ProposedMapping[] | null>(null);
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const [isProcessing, setIsProcessing] = useState(false);
  const [lastFile, setLastFile] = useState<string | null>(null);
  const dragCounter = useRef(0); // tracks nested enter/leave events

  // ── Core processing logic ──────────────────────────────────────────────────

  async function processFile(filePath: string) {
    let apiKey = localStorage.getItem("mergemark_openai_key");
    const baseUrl = localStorage.getItem("mergemark_openai_base_url") || "https://api.openai.com/v1";
    const modelName = localStorage.getItem("mergemark_openai_model") || "gpt-4o-mini";

    // If it's a local base URL (like Ollama) we can tolerate an empty API key, otherwise default to a dummy if they left it blank but still try to proceed, though it'll likely fail at the provider level if they actually need one.
    // For simplicity, we just default to "dummy" if it's empty, allowing local Ollama to work without a key.
    if (!apiKey || apiKey.trim() === "") {
      apiKey = "dummy";
    }

    setLastFile(filePath);
    setIsProcessing(true);
    try {
      let pdfBase64Pages: string[] | undefined = undefined;
      
      if (filePath.toLowerCase().endsWith(".pdf")) {
        const assetUrl = convertFileSrc(filePath);
        const response = await fetch(assetUrl);
        const arrayBuffer = await response.arrayBuffer();
        
        const pdf = await pdfjsLib.getDocument({ data: arrayBuffer }).promise;
        const pages: string[] = [];
        const numPages = pdf.numPages; // Process the entire document
        
        for (let i = 1; i <= numPages; i++) {
          const page = await pdf.getPage(i);
          const viewport = page.getViewport({ scale: 2.0 });
          const canvas = document.createElement("canvas");
          const context = canvas.getContext("2d");
          if (context) {
            canvas.width = viewport.width;
            canvas.height = viewport.height;
            await page.render({ canvasContext: context, canvas, viewport }).promise;
            const dataUrl = canvas.toDataURL("image/jpeg", 0.9);
            pages.push(dataUrl.split(",")[1]);
          }
        }
        pdfBase64Pages = pages;
      }

      if (importMode === "mark_scheme") {
        const mappings = await invoke<ProposedMapping[]>("parse_mark_scheme_vision", {
          filePath,
          apiKey,
          pdfBase64Pages,
          baseUrl,
          modelName
        });
        setPendingMappings(mappings);
      } else {
        const questions = await invoke<any[]>("parse_pdf_vision", { 
          filePath, 
          apiKey,
          pdfBase64Pages,
          baseUrl,
          modelName,
          subject
        });
        const count = questions.length;
        toast.success(
          count === 1 ? "1 question extracted!" : `${count} questions extracted!`,
          { description: filePath, duration: 6000 }
        );
        if (onSuccess) onSuccess();
      }
    } catch (err) {
      toast.error("Ingestion failed", {
        description: String(err),
        duration: 8000,
      });
    } finally {
      setIsProcessing(false);
    }
  }

  // ── Native file picker ─────────────────────────────────────────────────────

  async function handleBrowse() {
    if (isProcessing) return;
    const selected = await open({
      multiple: false,
      filters: [
        { name: "Documents & Images", extensions: ["pdf", "txt", "png", "jpg", "jpeg"] },
      ],
    });
    if (selected && typeof selected === "string") {
      await processFile(selected);
    }
  }

  // ── Drag-and-drop handlers ─────────────────────────────────────────────────

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current += 1;
    if (dragCounter.current === 1) setIsDraggingOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current -= 1;
    if (dragCounter.current === 0) setIsDraggingOver(false);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "copy";
  }, []);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      dragCounter.current = 0;
      setIsDraggingOver(false);
      if (isProcessing) return;

      const file = e.dataTransfer.files[0];
      if (!file) return;

      // On Tauri v2 the drag-and-drop event exposes the real FS path via
      // the non-standard `path` property on the File object.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const filePath: string | undefined = (file as any).path ?? file.name;
      if (filePath) {
        await processFile(filePath);
      }
    },
    [isProcessing]
  );

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <section
      className="flex flex-1 flex-col items-center justify-center h-full min-h-0 px-8 py-12 bg-background"
      aria-label="PDF Ingestion"
    >
      {/* Page heading */}
      <div className="mb-8 text-center space-y-1 select-none">
        <h1 className="text-2xl font-bold tracking-tight text-foreground">
          Import Document
        </h1>
        <p className="text-sm text-muted-foreground max-w-md">
          Drop a PDF, image, or plain-text document below and MergeMark will
          automatically process it into your repository.
        </p>
      </div>

      {/* ── Controls (Mode & Subject) ── */}
      <div className="flex flex-col items-center gap-4 mb-6">
        <div className="flex items-center justify-center bg-muted/30 p-1 rounded-lg border border-border/50">
          <button
            onClick={() => setImportMode("questions")}
            className={cn(
              "px-4 py-2 text-sm font-medium rounded-md transition-all duration-200",
              importMode === "questions" 
                ? "bg-background text-foreground shadow-sm ring-1 ring-border/50" 
                : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
            )}
          >
            Import Question Paper
          </button>
          <button
            onClick={() => setImportMode("mark_scheme")}
            className={cn(
              "px-4 py-2 text-sm font-medium rounded-md transition-all duration-200",
              importMode === "mark_scheme" 
                ? "bg-background text-foreground shadow-sm ring-1 ring-border/50" 
                : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
            )}
          >
            Import Mark Scheme
          </button>
        </div>

        <div className="flex items-center gap-3">
          <label htmlFor="subject-select" className="text-sm font-medium text-foreground">
            Paper Subject:
          </label>
          <select
            id="subject-select"
            value={subject}
            onChange={(e) => setSubject(e.target.value)}
            disabled={isProcessing}
            className="h-9 w-48 rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          >
            <option value="Mathematics">Mathematics</option>
            <option value="Further Mathematics">Further Mathematics</option>
            <option value="Physics">Physics</option>
            <option value="Computer Science">Computer Science</option>
          </select>
        </div>
      </div>

      {/* ── Dropzone card ── */}
      <button
        id="ingestion-dropzone"
        type="button"
        onClick={handleBrowse}
        onDragEnter={handleDragEnter}
        onDragLeave={handleDragLeave}
        onDragOver={handleDragOver}
        onDrop={handleDrop}
        disabled={isProcessing}
        aria-label="Drag and drop a file here, or click to browse"
        className={cn(
          // Layout
          "relative flex flex-col items-center justify-center gap-5",
          "w-full max-w-xl h-72 rounded-2xl",
          // Border — dashed, animated colour shift on hover / drag
          "border-2 border-dashed transition-all duration-300 ease-out",
          // Base state
          "border-border/60 bg-muted/20",
          // Hover (when not processing)
          !isProcessing && "hover:border-primary/60 hover:bg-primary/5 hover:shadow-lg hover:shadow-primary/10 cursor-pointer",
          // Active drag-over state
          isDraggingOver && "border-primary bg-primary/10 shadow-xl shadow-primary/20 scale-[1.015]",
          // Processing state
          isProcessing && "cursor-not-allowed opacity-70",
          // Focus ring for keyboard users
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
        )}
      >
        {/* Subtle radial glow behind icon */}
        <div
          className={cn(
            "absolute inset-0 rounded-2xl transition-opacity duration-300",
            "bg-[radial-gradient(ellipse_at_center,hsl(var(--primary)/0.06)_0%,transparent_70%)]",
            isDraggingOver ? "opacity-100" : "opacity-0"
          )}
          aria-hidden
        />

        {/* Icon */}
        <div
          className={cn(
            "relative flex items-center justify-center rounded-full p-5",
            "border border-border/60 bg-muted/40 transition-all duration-300",
            isDraggingOver && "border-primary/50 bg-primary/10 shadow-md shadow-primary/20"
          )}
        >
          {isProcessing ? (
            <Loader2 className="size-10 text-primary animate-spin" />
          ) : (
            <UploadCloud
              className={cn(
                "size-10 transition-colors duration-300",
                isDraggingOver ? "text-primary" : "text-muted-foreground"
              )}
            />
          )}
        </div>

        {/* Label text */}
        <div className="relative text-center space-y-1 px-4">
          {isProcessing ? (
            <>
              <p className="text-base font-semibold text-foreground">
                Processing…
              </p>
              <p className="text-xs text-muted-foreground truncate max-w-xs">
                {lastFile ?? ""}
              </p>
            </>
          ) : (
            <>
              <p className="text-base font-semibold text-foreground">
                {isDraggingOver
                  ? "Release to import"
                  : `Drag & Drop a ${importMode === "questions" ? "Past Paper" : "Mark Scheme"} here`}
              </p>
              <p className="text-xs text-muted-foreground">
                or{" "}
                <span className="text-primary font-medium underline underline-offset-2">
                  click to browse
                </span>{" "}
                · PDF, Image, or TXT accepted
              </p>
            </>
          )}
        </div>

        {/* Animated border pulse when dragging */}
        {isDraggingOver && (
          <span
            className="absolute inset-0 rounded-2xl border-2 border-primary animate-ping opacity-20 pointer-events-none"
            aria-hidden
          />
        )}
      </button>

      {/* Accepted formats hint */}
      <div className="mt-6 flex items-center gap-2 text-xs text-muted-foreground select-none">
        <FileText className="size-3.5 opacity-60" aria-hidden />
        <span>Accepted: .pdf, .png, .jpg, .txt</span>
      </div>

      <ReviewSyncModal
        mappings={pendingMappings}
        onClose={() => setPendingMappings(null)}
        onSuccess={() => {
          setPendingMappings(null);
          if (onSuccess) onSuccess();
        }}
      />
    </section>
  );
}
