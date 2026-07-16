import { useState } from "react";
import { Toaster } from "sonner";
import { LayoutGrid, UploadCloud, Settings as SettingsIcon } from "lucide-react";
import { RepositoryFeed } from "@/components/repository/RepositoryFeed";
import { WorksheetBuilder } from "@/components/worksheet/WorksheetBuilder";
import { IngestionDropzone } from "@/components/ingestion/IngestionDropzone";
import { Settings } from "@/components/settings/Settings";
import { type QuestionCardProps } from "@/components/repository/QuestionCard";
import { type WorksheetItemData } from "@/components/worksheet/WorksheetItem";
import { cn } from "@/lib/utils";

export type SelectedQuestion = Omit<QuestionCardProps, "onAddToWorksheet">;

type Tab = "repository" | "ingestion" | "settings";

const TABS: { id: Tab; label: string; icon: React.ElementType }[] = [
  { id: "repository", label: "Repository", icon: LayoutGrid },
  { id: "ingestion", label: "Import PDF", icon: UploadCloud },
  { id: "settings", label: "Settings", icon: SettingsIcon },
];

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("repository");
  const [selectedQuestions, setSelectedQuestions] = useState<WorksheetItemData[]>([]);

  function handleAddQuestion(question: SelectedQuestion) {
    setSelectedQuestions((prev) => {
      if (prev.some((q) => q.id === question.id)) return prev;

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

  function handleReorderQuestions(newQuestions: WorksheetItemData[]) {
    setSelectedQuestions(newQuestions);
  }

  return (
    <div className="flex h-screen w-full overflow-hidden bg-background text-foreground">
      {/* ── Left: tabbed main area ── */}
      <div className="flex flex-col flex-1 min-w-0 overflow-hidden">

        {/* Tab bar */}
        <nav
          className="flex items-center gap-1 border-b border-border px-4 pt-2 bg-background/80 backdrop-blur-sm"
          aria-label="Main navigation"
        >
          {TABS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              id={`tab-${id}`}
              type="button"
              role="tab"
              aria-selected={activeTab === id}
              onClick={() => setActiveTab(id)}
              className={cn(
                "flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-t-lg",
                "transition-colors duration-150 border-b-2 -mb-px",
                activeTab === id
                  ? "border-primary text-primary bg-primary/5"
                  : "border-transparent text-muted-foreground hover:text-foreground hover:bg-muted/40"
              )}
            >
              <Icon className="size-4" aria-hidden />
              {label}
            </button>
          ))}
        </nav>

        {/* Tab panels */}
        <div className="flex flex-col flex-1 min-h-0 overflow-hidden relative">
          <div className={cn("absolute inset-0 flex flex-col min-h-0 overflow-hidden bg-background", activeTab === "repository" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
            <RepositoryFeed isActive={activeTab === "repository"} onAddToWorksheet={handleAddQuestion} />
          </div>
          <div className={cn("absolute inset-0 flex flex-col min-h-0 overflow-hidden bg-background", activeTab === "ingestion" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
            <IngestionDropzone onSuccess={() => setActiveTab("repository")} />
          </div>
          <div className={cn("absolute inset-0 flex flex-col min-h-0 overflow-hidden bg-background", activeTab === "settings" ? "z-10 opacity-100 pointer-events-auto" : "z-0 opacity-0 pointer-events-none")}>
            <Settings />
          </div>
        </div>
      </div>

      {/* ── Right: worksheet builder (always visible) ── */}
      <WorksheetBuilder
        selectedQuestions={selectedQuestions}
        onRemove={handleRemoveQuestion}
        onReorder={handleReorderQuestions}
      />

      {/* Global toast notifications */}
      <Toaster theme="dark" richColors position="bottom-right" />
    </div>
  );
}

export default App;
