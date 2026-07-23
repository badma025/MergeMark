import { useState, useCallback, useRef, useEffect } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ReviewSyncModal, type ProposedMapping } from "./ReviewSyncModal";
import { open } from "@tauri-apps/plugin-dialog";
import { notifyUsageChanged } from "@/components/UploadCounter";
import { toast } from "sonner";
import { UploadCloud, FileText, Loader2, AlertTriangle, CheckCircle2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { useTaxonomy } from "@/lib/TaxonomyContext";

import * as pdfjsLib from "pdfjs-dist";

pdfjsLib.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.mjs",
  import.meta.url
).toString();

export interface Quarantine { scope: string; page?: number; questionNumber?: number; reason: string }
export interface TimingEntry {
  stage: string;
  operation: string;
  page?: number;
  questionNumber?: number;
  milliseconds: number;
}
export interface ImportReport {
  paperName: string;
  kind: string;
  pagesTotal: number;
  questionsExpected: number;
  questionsExtracted: number;
  marksChecksumOk: boolean | null;
  quarantined: Quarantine[];
  repairs: number;
  salvageEvents: number;
  cropRejections: number;
  diagramsSaved: number;
  diagramsDeduped: number;
  anomalies: string[];
  timings: TimingEntry[];
}

// ── IngestionDropzone ─────────────────────────────────────────────────────────

interface IngestionDropzoneProps {
  isActive?: boolean;
  onSuccess?: () => void;
}

export function IngestionDropzone({ isActive = false, onSuccess }: IngestionDropzoneProps) {
  const { subjects } = useTaxonomy();
  const [importMode, setImportMode] = useState<"questions" | "mark_scheme">("questions");
  const [subject, setSubject] = useState("");
  const [moduleOverride, setModuleOverride] = useState("");

  useEffect(() => {
    if (subjects.length > 0 && !subject) {
      setSubject(subjects[0].id);
    }
  }, [subjects, subject]);
  // Paper names already in the DB — populated when the user switches to mark_scheme mode.
  const [availablePaperNames, setAvailablePaperNames] = useState<string[]>([]);
  // The paper name the user has selected to match the mark scheme against.
  const [msPaperName, setMsPaperName] = useState("");
  const [pendingMappings, setPendingMappings] = useState<ProposedMapping[] | null>(null);
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const [isProcessing, setIsProcessing] = useState(false);
  const [progressMsg, setProgressMsg] = useState("");
  const [lastFile, setLastFile] = useState<string | null>(null);
  const [reports, setReports] = useState<ImportReport[]>([]);
  const [showLogs, setShowLogs] = useState(false);
  
  const activeSubject = subjects.find(s => s.id === subject);
  const availableModules = activeSubject ? activeSubject.modules : [];

  useEffect(() => {
    if (availableModules.length > 0) {
      // Only set if current moduleOverride is not in the new available list
      if (!availableModules.find((m: any) => m.id === moduleOverride)) {
        setModuleOverride(availableModules[0].id);
      }
    } else {
      setModuleOverride("");
    }
  }, [subject, availableModules]);

  const dragCounter = useRef(0); // tracks nested enter/leave events

  useEffect(() => {
    const unlisten = listen('import-progress', (event: any) => {
      setProgressMsg(event.payload.message);
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  // Structured completion report from the ingestion pipeline.
  useEffect(() => {
    // Helper to build human-readable timing summary
    function buildTimingSummary(timings: TimingEntry[]): string[] {
      const byStage = new Map<string, Map<string, number>>();
      for (const t of timings) {
        if (!byStage.has(t.stage)) byStage.set(t.stage, new Map());
        const ops = byStage.get(t.stage)!;
        ops.set(t.operation, (ops.get(t.operation) || 0) + t.milliseconds);
      }
      const summary: string[] = [];
      for (const [stage, ops] of byStage) {
        const parts: string[] = [];
        for (const [op, ms] of ops) {
          parts.push(`${op}: ${(ms / 1000).toFixed(1)}s`);
        }
        summary.push(`${stage} [${parts.join(", ")}]`);
      }
      return summary;
    }

    const unlisten = listen('import-report', (event: any) => {
      const r = event.payload as ImportReport;
      setReports(prev => [r, ...prev]);
      const warnings: number = r.quarantined.length;
      const checksumFailed = r.marksChecksumOk === false;
      
      // Build timing summary
      const timingSummary = buildTimingSummary(r.timings);
      const totalMs = r.timings.reduce((sum, t) => sum + t.milliseconds, 0);
      const timingStr = timingSummary.length > 0 
        ? `\n\nTiming: ${timingSummary.join(", ")} (total: ${(totalMs / 1000).toFixed(1)}s)`
        : "";
      
      if (warnings === 0 && !checksumFailed) {
        if (r.repairs > 0 || r.salvageEvents > 0 || r.cropRejections > 0) {
          toast.success("Import complete", {
            description: `${r.paperName}: all checks passed (${r.repairs} auto-repairs, ${r.salvageEvents} truncations salvaged, ${r.cropRejections} bad crops rejected).${timingStr}`,
            duration: 8000,
          });
        }
        return;
      }
      const parts: string[] = [];
      if (r.questionsExpected > 0) {
        parts.push(`${r.questionsExtracted}/${r.questionsExpected} questions extracted`);
      }
      if (checksumFailed) parts.push("marks don't match the printed paper total");
      if (warnings > 0) {
        const where = r.quarantined
          .slice(0, 3)
          .map(q => q.questionNumber ? `Q${q.questionNumber}` : q.page ? `page ${q.page}` : q.scope)
          .join(", ");
        const firstReason = r.quarantined[0]?.reason || "unknown";
        parts.push(`${warnings} item${warnings > 1 ? "s" : ""} quarantined (${where}). First error: ${firstReason}`);
      }
      toast.warning("Import finished with warnings", {
        description: `${r.paperName}: ${parts.join(" · ")}. Review flagged cards before building worksheets.${timingStr}`,
        duration: 15000,
      });
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  // Fetch distinct paper names from the DB whenever the user enters mark_scheme mode.
  useEffect(() => {
    if (importMode !== "mark_scheme") return;
    invoke<string[]>("get_paper_names")
      .then((names) => {
        setAvailablePaperNames(names);
        // Auto-select the first one if nothing is selected yet.
        if (msPaperName === "" && names.length > 0) {
          setMsPaperName(names[0]);
        }
      })
      .catch(() => setAvailablePaperNames([]));
  }, [importMode]);

  // ── Core processing logic ──────────────────────────────────────────────────

  async function processFile(filePath: string) {
    let apiKey = localStorage.getItem("mergemark_openai_key");
    const baseUrl = localStorage.getItem("mergemark_openai_base_url") || "https://openrouter.ai/api/v1/";
    const modelName = localStorage.getItem("mergemark_openai_model") || "google/gemini-2.5-flash";

    // If it's a local base URL (like Ollama) we can tolerate an empty API key, otherwise default to a dummy if they left it blank but still try to proceed, though it'll likely fail at the provider level if they actually need one.
    // For simplicity, we just default to "dummy" if it's empty, allowing local Ollama to work without a key.
    if (!apiKey || apiKey.trim() === "") {
      apiKey = "dummy";
    }

    // Derive the paper name from the file's basename (minus extension).
    // e.g. "C:\\exams\\2024_June_P1.pdf" → "2024_June_P1"
    const paperName =
      filePath.replace(/\\/g, "/").split("/").pop()?.replace(/\.[^.]+$/, "") ||
      filePath;

    setLastFile(filePath);
    setIsProcessing(true);
    setProgressMsg("");
    try {
      let pdfBase64Pages: string[] | undefined = undefined;
      
      if (filePath.toLowerCase().endsWith(".pdf")) {
        const assetUrl = convertFileSrc(filePath);
        const response = await fetch(assetUrl);
        const arrayBuffer = await response.arrayBuffer();

        const pdf = await pdfjsLib.getDocument({
          data: arrayBuffer,
          standardFontDataUrl: `https://unpkg.com/pdfjs-dist@${pdfjsLib.version}/standard_fonts/`,
          cMapUrl: `https://unpkg.com/pdfjs-dist@${pdfjsLib.version}/cmaps/`,
          cMapPacked: true,
          wasmUrl: `https://unpkg.com/pdfjs-dist@${pdfjsLib.version}/wasm/`,
        }).promise;
        const pages: string[] = [];
        const numPages = pdf.numPages;

        // Phase 0 render settings:
        //  * RENDER_SCALE  = 2.0  → ~200 DPI on a standard A4 page (72 DPI CSS
        //    pixel * 2 ≈ 144 DPI; pdf.js points are 1/72" so scale 2.0 gives
        //    144 DPI rendered pixels — enough for the vision model and for
        //    in-app diagram crops. Diagram crops from this buffer are ~2×
        //    sharper than the previous 0.6-scale (0.6 → 2.0 = 3.3× linear
        //    resolution, ~11× pixel count).
        //  * JPEG_QUALITY = 0.92 → visually lossless for text+line art;
        //    avoids the 0.82 JPEG mush on fine lines and subscripts.
        //  * Every non-blank page is rasterized. Previously the code sent the
        //    sentinel "TEXT_ONLY" for pages without embedded raster images,
        //    but PDF vector graphics (diagrams, circuits, graphs) are drawn
        //    as PATH operators — not images — so those pages were sent
        //    without a picture. The model then never saw the figure.
        const RENDER_SCALE = 2.0;
        const JPEG_QUALITY = 0.92;
        // Sentinel strings the Rust pipeline recognises (must stay in sync
        // with pipeline.rs::is_sentinel_page).
        const SKIP = "__SKIP__";
        const ops: any = pdfjsLib.OPS;
        // Vector-path operator set — any one of these on a page means the
        // page has drawn lines/curves (graphs, circuits, force diagrams,
        // geometry figures) and must be rendered as an image.
        const VECTOR_PATH_OPS = new Set<number>([
          ops.constructPath,         // 64
          ops.closePath,             // 67
          ops.stroke,                // 68
          ops.fill,                  // 69
          ops.eoFill,                // 70
          ops.fillStroke,            // 75
          ops.eoFillStroke,          // 76
          ops.rectangle,             // 65
          ops.moveTo,                // 30
          ops.lineTo,                // 31
          ops.curveTo,               // 32
          ops.curveTo2,              // 33
          ops.curveTo3,              // 34
          ops.appendRectangle,       // 35
        ]);

        for (let i = 1; i <= numPages; i++) {
          try {
            const page = await pdf.getPage(i);

            // Blank-page detection: treat pages with effectively no text
            // content as skipped — no need to send an image to the model.
            const textContent = await page.getTextContent();
            const rawText = textContent.items.map((item: any) => item.str).join("").trim();
            const isBlank =
              !rawText || rawText.replace(/\s+/g, "").toUpperCase() === "BLANKPAGE";

            // Decide whether the page has any visual content beyond plain
            // text: either embedded raster images OR vector path operators.
            const opList = await page.getOperatorList();
            const hasRasterImage = opList.fnArray.some(
              (fn: number) =>
                fn === ops.paintImageXObject ||
                fn === ops.paintJpegXObject ||
                fn === ops.paintXObject ||
                fn === ops.paintInlineImageXObject ||
                fn === ops.paintInlineImageXObjectGroup ||
                fn === ops.paintImageMaskXObject ||
                fn === ops.paintImageMaskXObjectGroup,
            );
            const hasVectorPaths = opList.fnArray.some((fn: number) =>
              VECTOR_PATH_OPS.has(fn),
            );

            if (isBlank) {
              pages.push(SKIP);
              continue;
            }

            // Always render the page. Text-only (no image, no paths) pages
            // could in principle skip rendering, but pdf.js's operator list
            // misses some vector primitives (e.g. shading patterns, Type 3
            // fonts drawn as paths) and the token cost of a 200 DPI JPEG is
            // small compared to the cost of misclassifying a figure page as
            // text-only. Physics papers in particular draw nearly every
            // diagram as paths; we can't afford to miss them.
            const viewport = page.getViewport({ scale: RENDER_SCALE });
            const canvas = document.createElement("canvas");
            const context = canvas.getContext("2d", { alpha: false });
            if (!context) {
              pages.push(SKIP);
              continue;
            }
            // Round to integer pixels to avoid subpixel bleed.
            canvas.width = Math.ceil(viewport.width);
            canvas.height = Math.ceil(viewport.height);
            context.fillStyle = "white";
            context.fillRect(0, 0, canvas.width, canvas.height);
            // We already filled the canvas with opaque white above, which
            // is the pdf.js-portable way to ensure transparent objects
            // composite onto white (they would otherwise JPEG-encode to
            // black). `intent: "display"` uses the device pixel ratio
            // without printing overprint effects.
            await page.render({
              canvasContext: context,
              canvas,
              viewport,
              intent: "display",
            }).promise;
            const dataUrl = canvas.toDataURL("image/jpeg", JPEG_QUALITY);
            const b64 = dataUrl.split(",")[1];
            pages.push(b64);
          } catch (pageErr) {
            console.error(`Error processing page ${i}:`, pageErr);
            pages.push("__SKIP__");
          }
        }
        pdfBase64Pages = pages;
      }

      if (importMode === "mark_scheme") {
        // Use the DB-selected paper name so MS questions match the QP that was already imported.
        const effectivePaperName = msPaperName;
        if (!effectivePaperName) {
          throw new Error("Please select which question paper this mark scheme belongs to before importing.");
        }
        const mappings = await invoke<ProposedMapping[]>("parse_mark_scheme_vision", {
          filePath,
          apiKey,
          pdfBase64Pages,
          baseUrl,
          modelName,
          paperName: effectivePaperName,
        });
        setPendingMappings(mappings);
      } else {
        const questions = await invoke<any[]>("parse_pdf_vision", { 
          filePath, 
          apiKey,
          pdfBase64Pages,
          baseUrl,
          modelName,
          subject,
          moduleOverride: moduleOverride.trim() !== "" ? moduleOverride.trim() : null,
          paperName,
        });
        const count = questions.length;
        toast.success(
          count === 1 ? "1 question extracted!" : `${count} questions extracted!`,
          { description: `Paper: ${paperName}`, duration: 6000 }
        );
        if (onSuccess) onSuccess();
      }
    } catch (err) {
      const errMsg = String(err);
      if (errMsg.includes("Import cancelled by user")) {
        toast.info("Import Cancelled", {
          description: `Stopped processing ${paperName}`,
          duration: 4000,
        });
      } else {
        toast.error("Ingestion failed", {
          description: errMsg,
          duration: 8000,
        });
      }
    } finally {
      notifyUsageChanged();
      setLastFile(null);
      setProgressMsg("");
      setIsProcessing(false);
    }
  }

  // ── Native Tauri Drag-and-Drop ─────────────────────────────────────────────

  const processFileRef = useRef(processFile);
  useEffect(() => {
    processFileRef.current = processFile;
  }, [processFile]);

  const isProcessingRef = useRef(isProcessing);
  useEffect(() => {
    isProcessingRef.current = isProcessing;
  }, [isProcessing]);

  const isActiveRef = useRef(isActive);
  useEffect(() => {
    isActiveRef.current = isActive;
  }, [isActive]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    
    import("@tauri-apps/api/webview").then(({ getCurrentWebview }) => {
      getCurrentWebview().onDragDropEvent((event) => {
        if (!isActiveRef.current) return;
        const payload = event.payload;
        if (payload.type === 'enter') {
          // Do nothing on global enter to prevent highlighting when dragging outside the zone
        } else if (payload.type === 'leave') {
          setIsDraggingOver(false);
          dragCounter.current = 0;
        } else if (payload.type === 'drop') {
          setIsDraggingOver(false);
          dragCounter.current = 0;
          const paths = payload.paths;
          if (paths && paths.length > 0 && !isProcessingRef.current) {
            // Check if dropped within the dropzone element bounding rect
            const dropzone = document.getElementById('ingestion-dropzone');
            if (dropzone && payload.position) {
              const rect = dropzone.getBoundingClientRect();
              const { x: rawX, y: rawY } = payload.position;
              const dpr = window.devicePixelRatio || 1;
              const logicalX = rawX / dpr;
              const logicalY = rawY / dpr;
              
              const isWithinRect = (px: number, py: number) => 
                px >= rect.left - 20 && px <= rect.right + 20 && py >= rect.top - 20 && py <= rect.bottom + 20;

              if (isWithinRect(rawX, rawY) || isWithinRect(logicalX, logicalY)) {
                processFileRef.current(paths[0]);
              }
            } else {
              // Fallback if position check fails
              processFileRef.current(paths[0]);
            }
          }
        }
      }).then(fn => {
        unlisten = fn;
      }).catch(console.error);
    }).catch(console.error);

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

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
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      dragCounter.current = 0;
      setIsDraggingOver(false);
      // The Tauri native 'tauri://drop' event will handle the actual file processing with absolute paths.
    },
    []
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

        <div className="flex flex-col gap-3">
          <div className="flex items-center gap-3">
            <label htmlFor="subject-select" className="text-sm font-medium text-foreground min-w-[100px]">
              Paper Subject:
            </label>
            <select
              id="subject-select"
              value={subject}
              onChange={(e) => setSubject(e.target.value)}
              disabled={isProcessing}
              className="h-9 w-[300px] rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
            >
              {subjects.map((s) => (
                <option key={s.id} value={s.id}>{s.name}</option>
              ))}
            </select>
          </div>

          {importMode === "questions" && availableModules.length > 0 && (
            <div className="flex items-center gap-3">
              <label htmlFor="module-override" className="text-sm font-medium text-foreground min-w-[100px]">
                Paper Module:
              </label>
              <select
                id="module-override"
                value={moduleOverride}
                onChange={(e) => setModuleOverride(e.target.value)}
                disabled={isProcessing}
                className="h-9 w-[300px] rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
              >
                {availableModules.map((m: any) => (
                  <option key={m.id} value={m.id}>{m.name}</option>
                ))}
              </select>
            </div>
          )}
        </div>

        {/* Match-to-paper selector — only shown when importing a mark scheme */}
        {importMode === "mark_scheme" && (
          <div className="flex items-center gap-3">
            <label htmlFor="ms-paper-select" className="text-sm font-medium text-foreground">
              Match to Paper:
            </label>
            {availablePaperNames.length === 0 ? (
              <p className="text-sm text-muted-foreground italic">
                No question papers imported yet.
              </p>
            ) : (
              <select
                id="ms-paper-select"
                value={msPaperName}
                onChange={(e) => setMsPaperName(e.target.value)}
                disabled={isProcessing}
                className="h-9 w-64 rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
              >
                {availablePaperNames.map((name) => (
                  <option key={name} value={name}>{name}</option>
                ))}
              </select>
            )}
          </div>
        )}


      </div>

      {/* ── Dropzone card ── */}
      <div
        id="ingestion-dropzone"
        role="button"
        tabIndex={isProcessing ? -1 : 0}
        onClick={handleBrowse}
        onDragEnter={handleDragEnter}
        onDragLeave={handleDragLeave}
        onDragOver={handleDragOver}
        onDrop={handleDrop}
        aria-disabled={isProcessing}
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
              <p className="text-xs text-primary truncate max-w-xs pb-1 font-medium">
                {progressMsg}
              </p>
              <p className="text-xs text-muted-foreground truncate max-w-xs pb-2">
                {lastFile ?? ""}
              </p>
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  invoke("cancel_import").catch(console.error);
                }}
                className="pointer-events-auto px-4 py-1.5 text-xs font-semibold text-destructive-foreground bg-destructive hover:bg-destructive/90 rounded-md transition-colors shadow-sm"
              >
                Cancel Import
              </button>
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
      </div>

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

      {/* ── Import Logs Modal ── */}
      {reports.length > 0 && (
        <div className="mt-8 select-none">
          <Button variant="outline" onClick={() => setShowLogs(true)} className="gap-2">
            <FileText className="size-4" />
            View Import Logs ({reports.length})
          </Button>

          <Dialog open={showLogs} onOpenChange={setShowLogs}>
            <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col p-6">
              <DialogHeader>
                <DialogTitle>Import Logs</DialogTitle>
              </DialogHeader>
              <div className="flex-1 overflow-y-auto space-y-4 pr-2 py-2">
                {reports.map((r, i) => {
                  const hasWarnings = r.quarantined.length > 0 || r.marksChecksumOk === false;
                  return (
                    <div key={i} className="border border-border/50 rounded-lg p-4 bg-muted/20">
                      <div className="flex items-center gap-2 mb-2">
                        {hasWarnings ? (
                          <AlertTriangle className="size-5 text-yellow-500" />
                        ) : (
                          <CheckCircle2 className="size-5 text-green-500" />
                        )}
                        <h3 className="font-semibold text-foreground text-lg">{r.paperName}</h3>
                      </div>
                      
                      <p className="text-sm text-muted-foreground mb-4">
                        {r.questionsExtracted}/{r.questionsExpected} questions extracted.
                        {r.marksChecksumOk === false && " Marks don't match the printed paper total."}
                      </p>

                      {r.quarantined.length > 0 && (
                        <div className="mb-4">
                          <span className="text-xs font-semibold text-destructive uppercase tracking-wider">Quarantined ({r.quarantined.length})</span>
                          <ul className="mt-2 space-y-2">
                            {r.quarantined.map((q, idx) => (
                              <li key={idx} className="text-sm text-foreground bg-destructive/10 px-3 py-2 rounded-md border border-destructive/20">
                                <span className="font-medium mr-2">{q.questionNumber ? `Q${q.questionNumber}` : q.page ? `Page ${q.page}` : q.scope}:</span>
                                {q.reason}
                              </li>
                            ))}
                          </ul>
                        </div>
                      )}

                      {r.anomalies && r.anomalies.length > 0 && (
                        <div className="mb-4">
                          <span className="text-xs font-semibold text-yellow-500 uppercase tracking-wider">Anomalies ({r.anomalies.length})</span>
                          <ul className="mt-2 space-y-2">
                            {r.anomalies.map((a, idx) => (
                              <li key={idx} className="text-sm text-foreground bg-yellow-500/10 px-3 py-2 rounded-md border border-yellow-500/20">
                                {a}
                              </li>
                            ))}
                          </ul>
                        </div>
                      )}

                      <div className="text-xs font-medium text-muted-foreground/70 pt-2 border-t border-border/50 flex flex-wrap gap-x-4 gap-y-1">
                        <span>Repairs: {r.repairs}</span>
                        <span>Salvaged: {r.salvageEvents}</span>
                        <span>Crops Rejected: {r.cropRejections}</span>
                      </div>
                    </div>
                  );
                })}
              </div>
            </DialogContent>
          </Dialog>
        </div>
      )}
    </section>
  );
}
