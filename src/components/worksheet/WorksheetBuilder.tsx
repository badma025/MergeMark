import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable";
import { restrictToVerticalAxis, restrictToParentElement } from "@dnd-kit/modifiers";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { FileText, Clock, Hash, Loader2, X, Trash2, Target, SlidersHorizontal, CheckCircle, GraduationCap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "@/components/ui/dialog";
import { WorksheetItem, type WorksheetItemData } from "./WorksheetItem";
import { cn } from "@/lib/utils";

// ── Stat badge ────────────────────────────────────────────────────────────────

function StatChip({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ElementType;
  label: string;
  value: string;
}) {
  return (
    <div
      className="flex flex-1 items-center gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2"
      aria-label={label}
    >
      <Icon className="size-3.5 flex-shrink-0 text-primary" aria-hidden />
      <div className="flex flex-col gap-0">
        <span className="text-[0.6rem] uppercase tracking-widest text-muted-foreground leading-none">
          {label}
        </span>
        <span className="text-sm font-bold text-foreground leading-tight">{value}</span>
      </div>
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export interface WorksheetBuilderProps {
  selectedQuestions: WorksheetItemData[];
  onRemove: (id: string) => void;
  onReorder: (newItems: WorksheetItemData[]) => void;
  onClear?: () => void;
  onClose?: () => void;
  className?: string;
}

export function WorksheetBuilder({ selectedQuestions, onRemove, onReorder, onClear, onClose, className }: WorksheetBuilderProps) {
  const totalMarks = selectedQuestions.reduce((acc, q) => acc + q.marks, 0);
  const estMinutes = Math.round(totalMarks * 1.2); // 1.2 min per mark standard exam heuristic
  const [targetMarks, setTargetMarks] = useState<number>(50);
  const [isEditingTarget, setIsEditingTarget] = useState(false);
  const [isCompiling, setIsCompiling] = useState(false);
  const [showPdfLatexError, setShowPdfLatexError] = useState(false);
  const [showCustomizer, setShowCustomizer] = useState(false);

  // Exam Customizer State
  const [examTitle, setExamTitle] = useState("Practice Examination Paper");
  const [schoolName, setSchoolName] = useState("");
  const [subject, setSubject] = useState("");
  const [timeAllowed, setTimeAllowed] = useState<number>(estMinutes);
  const [includeCoverPage, setIncludeCoverPage] = useState(true);
  const [answerLayout, setAnswerLayout] = useState<"compact" | "lined">("lined");
  const [instructions, setInstructions] = useState(
    "Answer all questions in the spaces provided.\nShow all necessary working out clearly.\nCalculators may be used where appropriate."
  );

  const progressPercent = Math.min(Math.round((totalMarks / (targetMarks || 1)) * 100), 100);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    if (over && active.id !== over.id) {
      const oldIndex = selectedQuestions.findIndex((i) => i.id === active.id);
      const newIndex = selectedQuestions.findIndex((i) => i.id === over.id);
      onReorder(arrayMove(selectedQuestions, oldIndex, newIndex));
    }
  }

  async function handleCompile() {
    const ids = selectedQuestions.map((q) => q.id);
    setIsCompiling(true);
    const effectiveFileName = examTitle.trim().replace(/\s+/g, "_") || "practice_paper";
    try {
      const filePaths = await invoke<string[]>("compile_worksheet", {
        questionIds: ids,
        fileName: effectiveFileName,
        options: {
          fileName: effectiveFileName,
          examTitle: examTitle.trim() || "Practice Paper",
          subject: subject.trim(),
          schoolName: schoolName.trim(),
          timeAllowedMins: timeAllowed || estMinutes,
          instructions: instructions.trim(),
          includeCoverPage,
          answerLayout,
        },
      });
      toast.success("Worksheet & Answer Key compiled!", {
        description: filePaths.join("\n"),
        duration: 8000,
      });
      setShowCustomizer(false);
    } catch (err) {
      if (err === "PDFLATEX_NOT_FOUND") {
        setShowPdfLatexError(true);
      } else {
        toast.error("Compilation failed", {
          description: String(err),
          duration: 8000,
        });
      }
    } finally {
      setIsCompiling(false);
    }
  }

  async function handleExportMarkdown() {
    const ids = selectedQuestions.map((q) => q.id);
    try {
      const markdown = await invoke<string>("export_worksheet_markdown", {
        questionIds: ids,
        options: {
          examTitle: examTitle.trim() || "Worksheet",
          subject: subject.trim(),
          timeAllowedMins: timeAllowed || estMinutes,
        },
      });
      const name = examTitle.trim().replace(/\s+/g, "_") || "worksheet";
      
      const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `${name}.md`;
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      URL.revokeObjectURL(url);
      
      toast.success("Markdown exported!");
      setShowPdfLatexError(false);
      setShowCustomizer(false);
    } catch (err) {
      toast.error("Markdown export failed", {
        description: String(err)
      });
    }
  }

  return (
    <aside
      className={cn(
        "flex w-full h-full flex-col border-l border-border bg-background min-h-0 overflow-hidden",
        className
      )}
      aria-label="Worksheet Builder"
    >
      {/* ── Header ── */}
      <div className="flex items-center justify-between border-b border-border px-4 py-3 shrink-0">
        <div className="flex flex-col gap-0.5 min-w-0">
          <div className="flex items-center gap-2">
            <FileText className="size-4 text-primary flex-shrink-0" />
            <h2 className="text-sm font-semibold tracking-tight text-foreground truncate">
              Current Worksheet
            </h2>
          </div>
          <p className="text-[0.7rem] text-muted-foreground pl-6">
            {selectedQuestions.length} question{selectedQuestions.length !== 1 ? "s" : ""}
          </p>
        </div>
        <div className="flex items-center gap-1">
          {selectedQuestions.length > 0 && onClear && (
            <Button
              variant="ghost"
              size="sm"
              onClick={onClear}
              className="h-7 text-xs text-muted-foreground hover:text-destructive hover:bg-destructive/10 gap-1 px-2"
              title="Clear all questions"
            >
              <Trash2 className="size-3" />
              <span>Clear</span>
            </Button>
          )}
          {onClose && (
            <Button
              variant="ghost"
              size="icon"
              onClick={onClose}
              className="size-8 rounded-lg text-muted-foreground hover:text-foreground shrink-0"
              aria-label="Close worksheet builder drawer"
            >
              <X className="size-4" />
            </Button>
          )}
        </div>
      </div>

      {/* ── Stats row ── */}
      <div className="flex gap-2 px-4 py-2.5 border-b border-border shrink-0">
        <StatChip icon={Hash} label="Total Marks" value={`${totalMarks}`} />
        <StatChip icon={Clock} label="Est. Time" value={`${estMinutes}m`} />
      </div>

      {/* ── Live Mark Budget Bar ── */}
      <div className="flex flex-col gap-1.5 px-4 py-2.5 bg-muted/30 border-b border-border/60 shrink-0">
        <div className="flex items-center justify-between text-xs">
          <div className="flex items-center gap-1.5 font-medium text-foreground">
            <Target className="size-3 text-muted-foreground" />
            <span>Target:</span>
            {isEditingTarget ? (
              <input
                type="number"
                min={1}
                max={200}
                value={targetMarks}
                autoFocus
                onBlur={() => setIsEditingTarget(false)}
                onChange={(e) => setTargetMarks(Math.max(1, parseInt(e.target.value) || 1))}
                className="w-14 px-1 py-0.5 text-xs bg-background border border-primary rounded text-foreground font-mono focus:outline-none"
              />
            ) : (
              <span 
                onClick={() => setIsEditingTarget(true)} 
                className="cursor-pointer font-mono font-bold hover:underline hover:text-primary transition-colors"
                title="Click to edit target marks"
              >
                {targetMarks} marks
              </span>
            )}
          </div>
          <span className={cn(
            "font-semibold font-mono text-[11px]",
            totalMarks > targetMarks ? "text-amber-500 dark:text-amber-400" : totalMarks === targetMarks ? "text-emerald-500" : "text-muted-foreground"
          )}>
            {totalMarks} / {targetMarks} ({Math.round((totalMarks / (targetMarks || 1)) * 100)}%)
          </span>
        </div>
        <div className="w-full h-1.5 bg-muted/80 rounded-full overflow-hidden">
          <div 
            className={cn(
              "h-full rounded-full transition-all duration-300",
              totalMarks > targetMarks ? "bg-amber-500" : totalMarks === targetMarks ? "bg-emerald-500" : "bg-primary"
            )}
            style={{ width: `${progressPercent}%` }}
          />
        </div>
      </div>

      {/* ── Scrollable sortable list ── */}
      <div className="flex-1 min-h-0 overflow-y-auto px-4 py-3">
        {selectedQuestions.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-32 gap-2 text-muted-foreground">
            <FileText className="size-8 opacity-25" />
            <p className="text-xs text-center">
              No questions yet.
              <br />
              Hit <span className="font-semibold text-primary">+</span> on any card to add one.
            </p>
          </div>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleDragEnd}
            modifiers={[restrictToVerticalAxis, restrictToParentElement]}
          >
            <SortableContext
              items={selectedQuestions.map((i) => i.id)}
              strategy={verticalListSortingStrategy}
            >
              <ul className="flex flex-col gap-2" aria-label="Worksheet questions">
                {selectedQuestions.map((item) => (
                  <WorksheetItem
                    key={item.id}
                    item={item}
                    onRemove={onRemove}
                  />
                ))}
              </ul>
            </SortableContext>
          </DndContext>
        )}
      </div>

      {/* ── Pinned compile button row ── */}
      <div className="border-t border-border px-4 py-3 bg-background flex flex-col gap-2 shrink-0">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setTimeAllowed(estMinutes);
              setShowCustomizer(true);
            }}
            disabled={selectedQuestions.length === 0}
            className="flex-1 gap-1.5 text-xs font-semibold h-9"
            title="Configure exam cover sheet, time limit, and answer layout"
          >
            <SlidersHorizontal className="size-3.5 text-primary" />
            <span>Customize Paper</span>
          </Button>
          <Button
            id="compile-pdf-btn"
            size="sm"
            className={cn(
              "flex-1 gap-1.5 text-xs font-semibold h-9",
              "bg-primary text-primary-foreground hover:bg-primary/90",
              "shadow-sm transition-all duration-150"
            )}
            onClick={handleCompile}
            disabled={selectedQuestions.length === 0 || isCompiling}
            aria-label="Compile worksheet to PDF"
          >
            {isCompiling ? (
              <>
                <Loader2 className="size-3.5 animate-spin" />
                <span>Compiling…</span>
              </>
            ) : (
              <>
                <FileText className="size-3.5" />
                <span>Compile PDF</span>
              </>
            )}
          </Button>
        </div>
      </div>
      
      {/* ── Exam Paper Customizer Dialog ── */}
      <Dialog open={showCustomizer} onOpenChange={setShowCustomizer}>
        <DialogContent className="sm:max-w-2xl max-w-[95vw] max-h-[88vh] flex flex-col p-0 gap-0 overflow-hidden rounded-xl border border-border shadow-2xl bg-card">
          {/* Dialog Header */}
          <div className="p-5 pb-3 border-b border-border/50 shrink-0">
            <DialogHeader className="gap-1">
              <DialogTitle className="flex items-center gap-2 text-base font-semibold">
                <GraduationCap className="size-5 text-primary" />
                <span>Exam Paper & Cover Sheet Settings</span>
              </DialogTitle>
              <DialogDescription className="text-xs text-muted-foreground">
                Customize the front cover page, exam instructions, timing, and student answer space layout.
              </DialogDescription>
            </DialogHeader>
          </div>

          {/* Dialog Scrollable Body */}
          <div className="flex-1 min-h-0 overflow-y-auto p-5 space-y-4">
            {/* Exam Title (Full Width) */}
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-semibold text-foreground">Exam Title</label>
              <input
                type="text"
                value={examTitle}
                onChange={(e) => setExamTitle(e.target.value)}
                placeholder="e.g. AQA Physics Paper 1 Mock Exam"
                className="w-full rounded-md border border-border bg-muted/30 px-3 py-2 text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-primary focus:border-primary transition-all"
              />
            </div>

            {/* Subject & School (2-column) */}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-semibold text-foreground">Subject / Course</label>
                <input
                  type="text"
                  value={subject}
                  onChange={(e) => setSubject(e.target.value)}
                  placeholder="e.g. A-Level Physics"
                  className="w-full rounded-md border border-border bg-muted/30 px-3 py-2 text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-primary focus:border-primary transition-all"
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-semibold text-foreground">School / Institution</label>
                <input
                  type="text"
                  value={schoolName}
                  onChange={(e) => setSchoolName(e.target.value)}
                  placeholder="e.g. MergeMark Academy (optional)"
                  className="w-full rounded-md border border-border bg-muted/30 px-3 py-2 text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-primary focus:border-primary transition-all"
                />
              </div>
            </div>

            {/* Time Allowed & Live Stats */}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-semibold text-foreground">Time Allowed (minutes)</label>
                <input
                  type="number"
                  min={1}
                  max={300}
                  value={timeAllowed}
                  onChange={(e) => setTimeAllowed(parseInt(e.target.value) || estMinutes)}
                  className="w-full rounded-md border border-border bg-muted/30 px-3 py-2 text-sm text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-primary focus:border-primary transition-all"
                />
              </div>
              <div className="flex flex-col gap-1.5 justify-end">
                <div className="p-2 px-3 rounded-md bg-muted/40 border border-border/60 text-xs flex items-center justify-between h-[38px]">
                  <span className="text-muted-foreground">Paper Stats:</span>
                  <span className="font-semibold font-mono text-foreground">
                    {totalMarks} Marks · ~{timeAllowed || estMinutes} Mins
                  </span>
                </div>
              </div>
            </div>

            {/* Layout Options */}
            <div className="flex flex-col gap-2 pt-1 border-t border-border/50">
              <label className="text-xs font-semibold text-foreground">Answer Space Layout</label>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div
                  onClick={() => setAnswerLayout("lined")}
                  className={cn(
                    "flex flex-col p-3 rounded-lg border cursor-pointer transition-all select-none",
                    answerLayout === "lined"
                      ? "border-primary bg-primary/10 text-foreground ring-1 ring-primary shadow-xs"
                      : "border-border bg-muted/20 text-muted-foreground hover:bg-muted/40"
                  )}
                >
                  <div className="flex items-center justify-between mb-1">
                    <span className="font-semibold text-xs text-foreground">Lined Exam Booklet</span>
                    {answerLayout === "lined" && <CheckCircle className="size-3.5 text-primary" />}
                  </div>
                  <p className="text-[11px] leading-snug">Official ruled answer lines scaled to question marks.</p>
                </div>

                <div
                  onClick={() => setAnswerLayout("compact")}
                  className={cn(
                    "flex flex-col p-3 rounded-lg border cursor-pointer transition-all select-none",
                    answerLayout === "compact"
                      ? "border-primary bg-primary/10 text-foreground ring-1 ring-primary shadow-xs"
                      : "border-border bg-muted/20 text-muted-foreground hover:bg-muted/40"
                  )}
                >
                  <div className="flex items-center justify-between mb-1">
                    <span className="font-semibold text-xs text-foreground">Compact Revision</span>
                    {answerLayout === "compact" && <CheckCircle className="size-3.5 text-primary" />}
                  </div>
                  <p className="text-[11px] leading-snug">Questions only without blank lined space. Saves pages.</p>
                </div>
              </div>
            </div>

            {/* Include Cover Page Toggle */}
            <div className="flex items-center gap-2 pt-1">
              <input
                id="include-cover-page-cb"
                type="checkbox"
                checked={includeCoverPage}
                onChange={(e) => setIncludeCoverPage(e.target.checked)}
                className="rounded border-border text-primary focus:ring-primary size-4 cursor-pointer"
              />
              <label htmlFor="include-cover-page-cb" className="text-xs font-medium text-foreground cursor-pointer select-none">
                Include formal Examination Cover Sheet (Candidate Name, Number, Score Box)
              </label>
            </div>

            {/* Exam Instructions */}
            {includeCoverPage && (
              <div className="flex flex-col gap-1.5 animate-in fade-in-50 duration-150">
                <label className="text-xs font-semibold text-foreground">Instructions to Candidates</label>
                <textarea
                  rows={3}
                  value={instructions}
                  onChange={(e) => setInstructions(e.target.value)}
                  className="w-full rounded-md border border-border bg-muted/30 p-2.5 text-xs text-foreground font-mono leading-relaxed focus:outline-none focus:ring-1 focus:ring-primary focus:border-primary resize-none"
                />
              </div>
            )}
          </div>

          {/* Dialog Footer */}
          <div className="p-4 px-5 border-t border-border/60 bg-muted/30 shrink-0 flex items-center justify-between gap-3">
            <Button variant="outline" size="sm" onClick={handleExportMarkdown} className="text-xs font-medium">
              Export Markdown
            </Button>
            <div className="flex items-center gap-2">
              <Button variant="ghost" size="sm" onClick={() => setShowCustomizer(false)} className="text-xs">
                Cancel
              </Button>
              <Button size="sm" onClick={handleCompile} disabled={isCompiling} className="text-xs font-semibold">
                {isCompiling ? <Loader2 className="size-3.5 animate-spin mr-1.5" /> : <FileText className="size-3.5 mr-1.5" />}
                Compile PDF & Mark Scheme
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* ── LaTeX / MiKTeX Error Dialog ── */}
      <Dialog open={showPdfLatexError} onOpenChange={setShowPdfLatexError}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>LaTeX Compilation Failed</DialogTitle>
            <DialogDescription>
              We couldn't find <strong>pdflatex</strong> on your system. MergeMark uses MiKTeX to compile worksheets into high-quality PDFs.
            </DialogDescription>
          </DialogHeader>
          <div className="text-sm text-muted-foreground my-2 space-y-2">
            <p>To compile to PDF, please install MiKTeX from <a href="https://miktex.org/download" target="_blank" rel="noreferrer" className="text-primary hover:underline">miktex.org</a> and restart MergeMark.</p>
            <p>Alternatively, you can export the raw Markdown and use your own workflow.</p>
          </div>
          <DialogFooter className="mt-4">
            <Button variant="outline" onClick={() => setShowPdfLatexError(false)}>Cancel</Button>
            <Button onClick={handleExportMarkdown}>Export as Markdown</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </aside>
  );
}
