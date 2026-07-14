import { Search } from "lucide-react";
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { QuestionCard, type QuestionCardProps } from "./QuestionCard";

// ── Component ─────────────────────────────────────────────────────────────────

export interface RepositoryFeedProps {
  onAddToWorksheet: (question: Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">) => void;
}

export function RepositoryFeed({ onAddToWorksheet }: RepositoryFeedProps) {
  const [search, setSearch] = useState("");
  const [questions, setQuestions] = useState<Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchQuestions();
  }, []);

  async function fetchQuestions() {
    setLoading(true);
    try {
      const data = await invoke<Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">[]>("get_all_questions");
      setQuestions(data);
    } catch (error) {
      console.error("Failed to fetch questions:", error);
      toast.error("Failed to load questions", { description: String(error) });
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete(id: string) {
    // Optimistically remove from local state immediately so the UI feels instant
    setQuestions((prev) => prev.filter((q) => q.id !== id));
    try {
      await invoke("delete_question", { id });
      toast.success("Question removed from repository");
    } catch (err) {
      // Roll back if the backend call fails
      toast.error("Failed to delete question", { description: String(err) });
      fetchQuestions(); // re-sync with DB
    }
  }

  async function handleUpdate(id: string, newContent: string, newMarks: number) {
    try {
      await invoke("update_question", { id, newContent, newMarks });
      setQuestions((prev) => 
        prev.map(q => q.id === id ? { ...q, content: newContent, marks: newMarks } : q)
      );
      toast.success("Question updated successfully");
    } catch (err) {
      toast.error("Failed to update question", { description: String(err) });
    }
  }

  async function handleDeleteAll() {
    if (!window.confirm("Are you sure you want to delete ALL questions? This cannot be undone.")) return;
    try {
      await invoke("delete_all_questions");
      setQuestions([]);
      toast.success("Repository cleared");
    } catch (err) {
      toast.error("Failed to clear repository", { description: String(err) });
    }
  }

  const filtered = questions.filter((q) => {
    const term = search.toLowerCase();
    return (
      term === "" ||
      q.subject.toLowerCase().includes(term) ||
      q.subtopic.toLowerCase().includes(term) ||
      q.content.toLowerCase().includes(term) ||
      q.mathSnippet.toLowerCase().includes(term)
    );
  });

  function handleAdd(id: string) {
    const question = questions.find((q) => q.id === id);
    if (question) {
      onAddToWorksheet(question);
    }
  }

  return (
    <section
      className="flex flex-col flex-1 min-w-0 overflow-hidden"
      aria-label="Question Repository"
    >
      {/* ── Sticky search bar & Controls ── */}
      <div className="sticky top-0 z-10 border-b border-border bg-background/80 backdrop-blur-sm px-6 py-3">
        <div className="flex items-center justify-between gap-4">
          <div className="relative max-w-xl flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
            <Input
              id="repository-search"
              type="search"
              placeholder="Search extracted questions..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="pl-9 bg-muted/40 border-border/60 focus-visible:bg-background"
              aria-label="Search questions"
            />
          </div>
          
          <div className="flex items-center gap-6">
            <span className="text-sm text-muted-foreground whitespace-nowrap">
              Total Questions: <span className="font-semibold text-foreground">{questions.length}</span>
            </span>
            <Button 
              variant="destructive" 
              size="sm" 
              onClick={handleDeleteAll}
              disabled={questions.length === 0}
              className="gap-2"
            >
              Clear Repository
            </Button>
          </div>
        </div>
      </div>

      {/* ── Scrollable question grid ── */}
      <div className="flex-1 overflow-y-auto px-6 py-5">
        {loading ? (
          <div className="flex flex-col items-center justify-center h-48 text-muted-foreground">
            <p className="text-sm">Loading questions...</p>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-48 text-muted-foreground gap-2">
            <Search className="size-8 opacity-30" />
            <p className="text-sm">No questions match your search.</p>
          </div>
        ) : (
          <ul
            className="grid gap-4 grid-cols-1 sm:grid-cols-2 xl:grid-cols-3"
            aria-label="Question cards"
          >
            {filtered.map((q) => (
              <li key={q.id}>
                <QuestionCard
                  {...q}
                  onAddToWorksheet={handleAdd}
                  onDelete={handleDelete}
                  onUpdate={handleUpdate}
                />
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
