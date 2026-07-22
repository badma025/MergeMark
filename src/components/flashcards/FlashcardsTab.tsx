import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { open } from "@tauri-apps/plugin-dialog";
import { Download, Upload, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { type WorksheetItemData } from "@/components/worksheet/WorksheetItem";

interface FlashcardsTabProps {
  selectedQuestions: WorksheetItemData[];
}

export function FlashcardsTab({ selectedQuestions }: FlashcardsTabProps) {
  const [isExporting, setIsExporting] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [fileName, setFileName] = useState("");

  async function handleExportFlashcards() {
    const ids = selectedQuestions.map((q) => q.id);
    setIsExporting(true);
    try {
      const filePath = await invoke<string>("export_flashcards", {
        questionIds: ids,
        fileName: fileName.trim(),
      });
      toast.success("Flashcards exported!", {
        description: filePath,
        duration: 8000,
      });
    } catch (err) {
      toast.error("Export failed", {
        description: String(err),
        duration: 8000,
      });
    } finally {
      setIsExporting(false);
    }
  }

  async function handleImportFlashcards() {
    try {
      const file = await open({
        multiple: false,
        filters: [{
          name: "Flashcards",
          extensions: ["csv", "txt", "tsv"]
        }]
      });

      if (file) {
        setIsImporting(true);
        const count = await invoke<number>("import_flashcards", {
          filePath: file,
        });
        toast.success(`Successfully imported ${count} flashcards`);
      }
    } catch (err) {
      toast.error("Failed to import flashcards", { description: String(err) });
    } finally {
      setIsImporting(false);
    }
  }

  return (
    <div className="flex-1 overflow-y-auto px-8 py-10 bg-background">
      <div className="max-w-2xl mx-auto flex flex-col gap-8">
        <div>
          <h2 className="text-2xl font-bold tracking-tight mb-2">Flashcard Integrations</h2>
          <p className="text-muted-foreground">
            Import and export questions to use in external flashcard apps like Anki and Quizlet.
          </p>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          {/* Export Card */}
          <div className="flex flex-col gap-4 p-6 border border-border rounded-xl bg-card shadow-sm">
            <div className="flex items-center gap-3">
              <div className="p-2.5 bg-primary/10 rounded-lg text-primary">
                <Download className="size-5" />
              </div>
              <h3 className="text-lg font-semibold">Export Flashcards</h3>
            </div>
            <p className="text-sm text-muted-foreground min-h-[40px]">
              Export the {selectedQuestions.length} question(s) currently in your worksheet to a CSV file.
            </p>
            
            <div className="flex flex-col gap-2 mt-auto">
              <input
                type="text"
                value={fileName}
                onChange={(e) => setFileName(e.target.value)}
                placeholder="File name (optional)"
                className="w-full rounded-md border border-border bg-muted/40 px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-primary text-foreground"
              />
              <Button
                onClick={handleExportFlashcards}
                disabled={selectedQuestions.length === 0 || isExporting}
                className="w-full gap-2 bg-primary text-primary-foreground hover:bg-primary/90"
              >
                {isExporting ? <Loader2 className="size-4 animate-spin" /> : <Download className="size-4" />}
                Export to CSV
              </Button>
            </div>
          </div>

          {/* Import Card */}
          <div className="flex flex-col gap-4 p-6 border border-border rounded-xl bg-card shadow-sm">
            <div className="flex items-center gap-3">
              <div className="p-2.5 bg-blue-500/10 rounded-lg text-blue-500">
                <Upload className="size-5" />
              </div>
              <h3 className="text-lg font-semibold">Import Flashcards</h3>
            </div>
            <p className="text-sm text-muted-foreground min-h-[40px]">
              Import flashcards from Anki (Notes in Plain Text) or Quizlet (CSV/TSV).
            </p>
            
            <div className="flex flex-col mt-auto pt-[42px]">
              <Button
                variant="outline"
                onClick={handleImportFlashcards}
                disabled={isImporting}
                className="w-full gap-2 transition-colors border-border hover:bg-muted/40"
              >
                {isImporting ? <Loader2 className="size-4 animate-spin" /> : <Upload className="size-4" />}
                Import CSV / TSV
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
