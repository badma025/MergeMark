import { useState, useEffect } from "react";
import { Toaster, toast } from "sonner";
import { LayoutGrid, UploadCloud, Settings as SettingsIcon, BookOpen, FileText } from "lucide-react";
import { RepositoryFeed } from "@/components/repository/RepositoryFeed";
import { WorksheetBuilder } from "@/components/worksheet/WorksheetBuilder";
import { IngestionDropzone } from "@/components/ingestion/IngestionDropzone";
import { Settings } from "@/components/settings/Settings";
import { type QuestionCardProps } from "@/components/repository/QuestionCard";
import { type WorksheetItemData } from "@/components/worksheet/WorksheetItem";
import { UploadCounter, useUploadCounter } from "@/components/UploadCounter";
import { TaxonomyProvider } from "@/lib/TaxonomyContext";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { FlashcardsTab } from "@/components/flashcards/FlashcardsTab";

export type SelectedQuestion = Omit<QuestionCardProps, "onAddToWorksheet">;

type Tab = "repository" | "ingestion" | "settings" | "flashcards";

const TABS: { id: Tab; label: string; icon: React.ElementType }[] = [
  { id: "repository", label: "Repository", icon: LayoutGrid },
  { id: "ingestion", label: "Import PDF", icon: UploadCloud },
  { id: "flashcards", label: "Flashcards", icon: BookOpen },
  { id: "settings", label: "Settings", icon: SettingsIcon },
];

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("repository");
  const [selectedQuestions, setSelectedQuestions] = useState<WorksheetItemData[]>([]);
  const [isWorksheetDrawerOpen, setIsWorksheetDrawerOpen] = useState(false);

  // Dismiss splash screen smoothly once the app shell has mounted
  useEffect(() => {
    const timer = setTimeout(() => {
      (window as unknown as { __dismissSplash?: () => void }).__dismissSplash?.();
    }, 60);
    return () => clearTimeout(timer);
  }, []);

  // ── Free-tier upload counter ──────────────────────────────────────────
  const { status: usageStatus, loading: usageLoading } = useUploadCounter();

  function handleAddQuestion(question: SelectedQuestion) {
    setSelectedQuestions((prev) => {
      if (prev.some((q) => q.id === question.id)) {
        return prev.filter((q) => q.id !== question.id);
      }

      const newWorksheetItem: WorksheetItemData = {
        id: question.id,
        subject: question.subject,
        subtopic: question.subtopic,
        marks: question.marks,
      };

      return [...prev, newWorksheetItem];
    });
  }

  function handleRemoveQuestion(id: string) {
    setSelectedQuestions((prev) => prev.filter((q) => q.id !== id));
  }

  function handleClearAllQuestions() {
    setSelectedQuestions([]);
    toast.success("Worksheet cleared");
  }

  function handleAddMultipleQuestions(questions: SelectedQuestion[]) {
    setSelectedQuestions((prev) => {
      const existingIds = new Set(prev.map((q) => q.id));
      const toAdd = questions
        .filter((q) => !existingIds.has(q.id))
        .map((q) => ({
          id: q.id,
          subject: q.subject,
          subtopic: q.subtopic,
          marks: q.marks,
        }));
      return [...prev, ...toAdd];
    });
    toast.success(`Added ${questions.length} questions to worksheet`);
  }

  function handleReorderQuestions(newQuestions: WorksheetItemData[]) {
    setSelectedQuestions(newQuestions);
  }

  return (
    <TaxonomyProvider>
      <div className="grid h-full w-full flex-1 grid-cols-1 lg:grid-cols-[1fr_350px] grid-rows-[1fr] overflow-hidden bg-background text-foreground">
        {/* ── Left / Main Content Column ── */}
        <div className="flex flex-col min-w-0 min-h-0 h-full overflow-hidden">
          {/* Top navigation header */}
          <nav
            className="flex items-center justify-between flex-wrap gap-2.5 border-b border-border px-4 py-2.5 bg-background/80 backdrop-blur-sm shrink-0"
            aria-label="Main navigation"
          >
            {/* Left: Brand Logo & Navigation Tabs */}
            <div className="flex items-center gap-3 sm:gap-6 flex-wrap min-w-0">
              <div className="flex items-center py-0.5 shrink-0">
                {/* Full logo for medium and larger viewports */}
                <img
                  src="/mergemark-full.svg"
                  alt="MergeMark Logo"
                  className="h-8 w-auto hidden md:block"
                />
                {/* Compact icon mark when viewport is thinner */}
                <img
                  src="/mergemark.svg"
                  alt="MergeMark Logo"
                  className="h-8 w-8 object-contain rounded-md block md:hidden"
                />
              </div>

              <div className="flex items-center gap-1 flex-wrap" role="tablist">
                {TABS.map(({ id, label, icon: Icon }) => (
                  <button
                    key={id}
                    id={`tab-${id}`}
                    type="button"
                    role="tab"
                    aria-selected={activeTab === id}
                    onClick={() => setActiveTab(id)}
                    className={cn(
                      "flex items-center gap-1.5 px-3 py-1.5 text-xs sm:text-sm font-medium rounded-lg",
                      "transition-colors duration-150 shrink-0",
                      activeTab === id
                        ? "bg-primary/10 text-primary font-semibold"
                        : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
                    )}
                  >
                    <Icon className="size-4 shrink-0" aria-hidden />
                    <span>{label}</span>
                  </button>
                ))}
              </div>
            </div>

            {/* Right: Counter & Mobile Worksheet Toggle */}
            <div className="flex items-center gap-2 shrink-0 ml-auto">
              <UploadCounter status={usageStatus} loading={usageLoading} />

              {/* Mobile/Tablet Drawer Toggle Button (only on screens < 1024px) */}
              <Button
                variant="outline"
                size="sm"
                onClick={() => setIsWorksheetDrawerOpen(true)}
                className="flex lg:hidden items-center gap-1.5 text-xs font-semibold h-8 relative shrink-0"
                aria-label="Open worksheet builder drawer"
              >
                <FileText className="size-3.5 text-primary shrink-0" />
                <span>Worksheet</span>
                {selectedQuestions.length > 0 && (
                  <span className="ml-0.5 inline-flex items-center justify-center bg-primary text-primary-foreground text-[10px] font-bold rounded-full size-4">
                    {selectedQuestions.length}
                  </span>
                )}
              </Button>
            </div>
          </nav>

          {/* Tab panels container */}
          <main className="flex-1 min-h-0 min-w-0 overflow-hidden relative">
            <div className={cn("absolute inset-0 flex flex-col min-h-0 min-w-0 overflow-hidden bg-background", activeTab === "repository" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
              <RepositoryFeed
                isActive={activeTab === "repository"}
                onAddToWorksheet={handleAddQuestion}
                onAddMultipleToWorksheet={handleAddMultipleQuestions}
                selectedQuestionIds={selectedQuestions.map((q) => q.id)}
              />
            </div>
            <div className={cn("absolute inset-0 flex flex-col min-h-0 min-w-0 overflow-hidden bg-background", activeTab === "ingestion" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
              <IngestionDropzone
                isActive={activeTab === "ingestion"}
                onSuccess={() => {
                  setActiveTab("repository");
                  setTimeout(() => window.dispatchEvent(new CustomEvent("refresh-questions")), 50);
                }}
              />
            </div>
            <div className={cn("absolute inset-0 flex flex-col min-h-0 min-w-0 overflow-hidden bg-background", activeTab === "flashcards" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
              <FlashcardsTab selectedQuestions={selectedQuestions} />
            </div>
            <div className={cn("absolute inset-0 flex flex-col min-h-0 min-w-0 overflow-hidden bg-background", activeTab === "settings" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
              <Settings />
            </div>
          </main>
        </div>

        {/* ── Right Column: Worksheet Builder on Desktop (>= 1024px) ── */}
        <div className="hidden lg:flex flex-col h-full w-full min-h-0 overflow-hidden">
          <WorksheetBuilder
            selectedQuestions={selectedQuestions}
            onRemove={handleRemoveQuestion}
            onReorder={handleReorderQuestions}
            onClear={handleClearAllQuestions}
          />
        </div>

        {/* ── Mobile/Tablet Slide-Over Drawer (< 1024px) ── */}
        {isWorksheetDrawerOpen && (
          <div className="fixed inset-0 z-50 flex justify-end lg:hidden" role="dialog" aria-modal="true" aria-label="Worksheet builder drawer">
            {/* Backdrop */}
            <div
              className="fixed inset-0 bg-black/60 backdrop-blur-xs transition-opacity animate-in fade-in"
              onClick={() => setIsWorksheetDrawerOpen(false)}
              aria-hidden="true"
            />
            {/* Drawer Panel */}
            <div className="relative z-10 w-[350px] max-w-[85vw] h-full bg-background shadow-2xl animate-in slide-in-from-right duration-200">
              <WorksheetBuilder
                selectedQuestions={selectedQuestions}
                onRemove={handleRemoveQuestion}
                onReorder={handleReorderQuestions}
                onClear={handleClearAllQuestions}
                onClose={() => setIsWorksheetDrawerOpen(false)}
              />
            </div>
          </div>
        )}

        {/* Global toast notifications */}
        <Toaster theme="dark" richColors position="bottom-right" />
      </div>
    </TaxonomyProvider>
  );
}

export default App;
