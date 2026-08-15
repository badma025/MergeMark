import { Search, Plus, X, FileText } from "lucide-react";
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { QuestionCard, type QuestionCardProps } from "./QuestionCard";
import { AddQuestionModal } from "./AddQuestionModal";
import { ManagePapersModal } from "./ManagePapersModal";
import { useTaxonomy } from "@/lib/TaxonomyContext";

// ── Component ─────────────────────────────────────────────────────────────────

export interface RepositoryFeedProps {
  isActive?: boolean;
  onAddToWorksheet: (question: Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">) => void;
  onAddMultipleToWorksheet?: (questions: Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">[]) => void;
  selectedQuestionIds?: string[];
}

export function RepositoryFeed({
  isActive = true,
  onAddToWorksheet,
  onAddMultipleToWorksheet,
  selectedQuestionIds = [],
}: RepositoryFeedProps) {
  const [search, setSearch] = useState("");
  const [showAddModal, setShowAddModal] = useState(false);
  const [showManagePapers, setShowManagePapers] = useState(false);
  const [selectedSubject, setSelectedSubject] = useState<string>("All");
  const [selectedModule, setSelectedModule] = useState<string>("All");
  const [selectedTopics, setSelectedTopics] = useState<string[]>([]);
  const [selectedPaper, setSelectedPaper] = useState<string>("All");
  const [selectedMarksRange, setSelectedMarksRange] = useState<"All" | "1-2" | "3-5" | "6+">("All");
  const [reviewFilter, setReviewFilter] = useState<"All" | "Clean" | "Needs review">("All");
  const [questions, setQuestions] = useState<Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">[]>([]);
  const [loading, setLoading] = useState(true);
  const { subjects, topicsBySubject } = useTaxonomy();
  const subjectNames = subjects.map(s => s.name);
  const ALL_TOPICS = Array.from(new Set(
    Object.values(topicsBySubject)
      .flatMap(subjectMods => Object.values(subjectMods).flat())
  ));

  const availablePapers = Array.from(
    new Set(
      questions
        .map((q) => (q as any).paperName)
        .filter((name): name is string => typeof name === "string" && name.trim().length > 0)
    )
  ).sort();

  useEffect(() => {
    if (isActive) {
      fetchQuestions();
    }
    const handleRefresh = () => {
      fetchQuestions();
    };
    window.addEventListener("refresh-questions", handleRefresh);
    return () => window.removeEventListener("refresh-questions", handleRefresh);
  }, [isActive]);

  async function fetchQuestions() {
    setLoading(true);
    try {
      const data = await invoke<Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">[]>("get_all_questions");
      setQuestions(data || []);
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

  async function handleUpdate(id: string, newContent: string, newMarks: number, newAnswerContent?: string, newTopics?: string[]) {
    try {
      const newTopicsStr = newTopics ? JSON.stringify(newTopics) : undefined;
      await invoke("update_question", { id, newContent, newMarks, newAnswerContent, newTopics: newTopicsStr });
      setQuestions((prev) => 
        prev.map(q => q.id === id ? { ...q, content: newContent, marks: newMarks, answerContent: newAnswerContent, topics: newTopicsStr ?? q.topics } : q)
      );
      toast.success("Question updated successfully");
    } catch (err) {
      toast.error("Failed to update question", { description: String(err) });
    }
  }

  const handleClearFilters = () => {
    setSearch("");
    setSelectedSubject("All");
    setSelectedModule("All");
    setSelectedTopics([]);
    setSelectedPaper("All");
    setSelectedMarksRange("All");
    setReviewFilter("All");
  };

  const hasActiveFilters =
    search !== "" ||
    selectedSubject !== "All" ||
    selectedModule !== "All" ||
    selectedTopics.length > 0 ||
    selectedPaper !== "All" ||
    selectedMarksRange !== "All" ||
    reviewFilter !== "All";

  const filtered = questions.filter((q) => {
    const term = search.toLowerCase().trim();
    const matchesSearch = term === "" ||
      (q.subject || "").toLowerCase().includes(term) ||
      (q.subtopic || "").toLowerCase().includes(term) ||
      (q.content || "").toLowerCase().includes(term) ||
      ((q as any).paperName || "").toLowerCase().includes(term) ||
      ((q as any).mathSnippet || "").toLowerCase().includes(term);

    const resolvedSubject = subjects.find(s => s.id === q.subject || s.name.toLowerCase() === (q.subject || "").toLowerCase())?.name || q.subject || "";
    const matchesSubject = selectedSubject === "All" || resolvedSubject.toLowerCase() === selectedSubject.toLowerCase();

    const qPaper = ((q as any).paperName || "").trim();
    const matchesPaper = selectedPaper === "All" || qPaper.toLowerCase() === selectedPaper.toLowerCase();

    let matchesMarks = true;
    if (selectedMarksRange === "1-2") {
      matchesMarks = q.marks >= 1 && q.marks <= 2;
    } else if (selectedMarksRange === "3-5") {
      matchesMarks = q.marks >= 3 && q.marks <= 5;
    } else if (selectedMarksRange === "6+") {
      matchesMarks = q.marks >= 6;
    }

    let matchesTopicFilter = true;
    if (selectedTopics.length > 0) {
      let qTopics: string[] = [];
      try {
        if (q.topics) {
          qTopics = JSON.parse(q.topics);
          if (!Array.isArray(qTopics)) qTopics = [];
        }
      } catch (e) {
        // ignore
      }
      matchesTopicFilter = qTopics.some((t) => selectedTopics.includes(t));
    }

    let matchesModuleFilter = true;
    if (selectedModule !== "All") {
      const qMod = (q as any).module;
      if (qMod && qMod !== "Unknown" && qMod !== "General") {
        matchesModuleFilter = qMod === selectedModule;
      } else {
        // Fallback to topics if no explicit module is provided
        const resolvedSubject = subjects.find(s => s.id === q.subject)?.name || q.subject;
        const moduleTopics = (topicsBySubject[resolvedSubject] || {})[selectedModule] || [];
        if (selectedTopics.length === 0) {
          let qTopics: string[] = [];
          try {
            if (q.topics) {
              qTopics = JSON.parse(q.topics);
              if (!Array.isArray(qTopics)) qTopics = [];
            }
          } catch (e) {}
          
          if (qTopics.length > 0) {
            matchesModuleFilter = qTopics.some((t) => moduleTopics.includes(t));
          }
        }
      }
    }

    let matchesReviewFilter = true;
    if (reviewFilter === "Clean") {
      matchesReviewFilter = !q.needsReview && !q.answerStale;
    } else if (reviewFilter === "Needs review") {
      matchesReviewFilter = !!q.needsReview || !!q.answerStale;
    }

    return matchesSearch && matchesTopicFilter && matchesSubject && matchesModuleFilter && matchesReviewFilter && matchesPaper && matchesMarks;
  });

  const totalMarksFiltered = filtered.reduce((sum, q) => sum + (q.marks || 0), 0);

  function handleAdd(id: string) {
    const question = questions.find((q) => q.id === id);
    if (question) {
      onAddToWorksheet(question);
    }
  }

  return (
    <section
      className="flex flex-col flex-1 h-full min-h-0 min-w-0 overflow-hidden"
      aria-label="Question Repository"
    >
      {/* ── Search bar & Controls ── */}
      <div className="shrink-0 border-b border-border bg-background/80 backdrop-blur-sm px-4 sm:px-6 py-3">
        <div className="flex flex-wrap items-center justify-between gap-3 sm:gap-4">
          {/* Search bar: drops to full width on < 768px screens */}
          <div className="relative w-full order-3 md:order-none md:flex-1 md:w-auto min-w-[200px] max-w-xl">
            <Search className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
            <Input
              id="repository-search"
              type="search"
              placeholder="Search extracted questions..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="pl-9 bg-muted/40 border-border/60 focus-visible:bg-background w-full"
              aria-label="Search questions"
            />
          </div>
          
          {/* Action buttons & filters */}
          <div className="flex flex-wrap items-center gap-2 sm:gap-3 order-1 md:order-none ml-auto md:ml-0">
            {/* Paper Filter */}
            {availablePapers.length > 0 && (
              <div className="w-[140px] sm:w-[160px]">
                <Select
                  value={selectedPaper}
                  onValueChange={(v) => {
                    if (v) setSelectedPaper(v);
                  }}
                >
                  <SelectTrigger className="h-8 text-xs font-semibold bg-muted/40">
                    <FileText className="size-3.5 mr-1.5 text-muted-foreground shrink-0" />
                    <SelectValue placeholder="All Papers" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="All">All Papers ({availablePapers.length})</SelectItem>
                    {availablePapers.map((p) => (
                      <SelectItem key={p} value={p}>{p}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Review status Filter */}
            <div className="w-[125px] sm:w-[135px]">
              <Select
                value={reviewFilter}
                onValueChange={(v) => {
                  if (v) setReviewFilter(v as "All" | "Clean" | "Needs review");
                }}
              >
                <SelectTrigger className="h-8 text-xs font-semibold bg-muted/40">
                  <SelectValue placeholder="Filter..." />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="All">All Items</SelectItem>
                  <SelectItem value="Clean">Clean</SelectItem>
                  <SelectItem value="Needs review">Needs review</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <Button
              onClick={() => setShowAddModal(true)}
              size="sm"
              className="gap-1.5 text-xs sm:text-sm h-8"
            >
              <Plus className="size-4" />
              <span className="hidden sm:inline">Add Question</span>
              <span className="sm:hidden">Add</span>
            </Button>
            <Button
              onClick={() => setShowManagePapers(true)}
              size="sm"
              variant="outline"
              className="gap-1.5 text-xs sm:text-sm h-8"
            >
              <span className="hidden sm:inline">Manage PDFs</span>
              <span className="sm:hidden">PDFs</span>
            </Button>
          </div>
        </div>
        
        {/* Subject Filter */}
        <div className="mt-3 flex flex-wrap items-center gap-2 border-b border-border/50 pb-3">
          {["All", ...subjectNames].map((subject) => {
            const isSelected = selectedSubject === subject;
            return (
              <Badge
                key={subject}
                variant={isSelected ? "default" : "secondary"}
                className={cn(
                  "cursor-pointer transition-colors text-xs font-semibold py-1 px-3 rounded-md break-normal",
                  isSelected ? "bg-primary text-primary-foreground hover:bg-primary/90" : "hover:bg-accent hover:text-accent-foreground border border-border/50"
                )}
                onClick={() => {
                  if (selectedSubject !== subject) {
                    setSelectedSubject(subject);
                    setSelectedModule("All");
                    setSelectedTopics([]);
                  }
                }}
              >
                {subject}
              </Badge>
            );
          })}
        </div>

        {/* Module Filter */}
        {selectedSubject !== "All" && (
          <div className="mt-3 flex flex-wrap items-center gap-2 border-b border-border/50 pb-3">
            {["All", ...Object.keys(topicsBySubject[selectedSubject] || {})].map((mod) => {
              const isSelected = selectedModule === mod;
              return (
                <Badge
                  key={mod}
                  variant={isSelected ? "default" : "secondary"}
                  className={cn(
                    "cursor-pointer transition-colors text-xs font-semibold py-1 px-3 rounded-md break-normal",
                    isSelected ? "bg-purple-600 text-white hover:bg-purple-700" : "hover:bg-accent hover:text-accent-foreground border border-border/50"
                  )}
                  onClick={() => {
                    if (selectedModule !== mod) {
                      setSelectedModule(mod);
                      setSelectedTopics([]);
                    }
                  }}
                >
                  {mod}
                </Badge>
              );
            })}
          </div>
        )}

        {/* Marks Range Pills & Active Filter Bar */}
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-b border-border/50 pb-3">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-xs font-medium text-muted-foreground mr-1">Marks:</span>
            {[
              { id: "All", label: "All Marks" },
              { id: "1-2", label: "1–2 (MCQ / Short)" },
              { id: "3-5", label: "3–5 (Standard)" },
              { id: "6+", label: "6+ (Extended)" },
            ].map(({ id, label }) => {
              const isSelected = selectedMarksRange === id;
              return (
                <Badge
                  key={id}
                  variant={isSelected ? "default" : "outline"}
                  className={cn(
                    "cursor-pointer text-xs font-medium py-0.5 px-2.5 rounded-md transition-all",
                    isSelected
                      ? "bg-emerald-600 text-white hover:bg-emerald-700 border-emerald-600"
                      : "hover:bg-accent border-border/60 text-muted-foreground hover:text-foreground"
                  )}
                  onClick={() => setSelectedMarksRange(id as any)}
                >
                  {label}
                </Badge>
              );
            })}
          </div>

          <div className="flex items-center gap-2.5 ml-auto flex-wrap">
            <span className="text-xs text-muted-foreground whitespace-nowrap">
              Showing <span className="font-semibold text-foreground">{filtered.length}</span> of {questions.length} questions ({totalMarksFiltered} marks)
            </span>
            {filtered.length > 0 && onAddMultipleToWorksheet && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => onAddMultipleToWorksheet(filtered)}
                className="h-6 text-xs font-semibold gap-1 px-2 text-primary hover:bg-primary/10 border-primary/30"
                title="Add all currently filtered questions to worksheet"
              >
                <Plus className="size-3" />
                <span>Add All ({filtered.length})</span>
              </Button>
            )}
            {hasActiveFilters && (
              <Button
                variant="ghost"
                size="sm"
                onClick={handleClearFilters}
                className="h-6 text-xs text-muted-foreground hover:text-foreground gap-1 px-2 hover:bg-destructive/10 hover:text-destructive"
              >
                <X className="size-3" />
                <span>Reset</span>
              </Button>
            )}
          </div>
        </div>

        {/* Topics Filter */}
        <div className="mt-3 flex flex-wrap items-center gap-2 max-h-36 overflow-y-auto pr-1">
          {(() => {
            if (selectedSubject === "All") return ALL_TOPICS;
            const subjectMods = topicsBySubject[selectedSubject] || {};
            if (selectedModule === "All") {
              return Object.values(subjectMods).flat();
            }
            return subjectMods[selectedModule] || [];
          })().map((topic) => {
            const isSelected = selectedTopics.includes(topic);
            return (
              <Badge
                key={topic}
                variant={isSelected ? "default" : "outline"}
                className={cn(
                  "cursor-pointer transition-colors text-xs font-medium py-0.5 break-normal",
                  isSelected ? "bg-blue-600 hover:bg-blue-700 text-white border-blue-600" : "hover:bg-accent border-border"
                )}
                onClick={() => {
                  setSelectedTopics(prev => 
                    prev.includes(topic) 
                      ? prev.filter(t => t !== topic)
                      : [...prev, topic]
                  );
                }}
              >
                {topic}
              </Badge>
            );
          })}
        </div>
      </div>

      {/* ── Scrollable question grid ── */}
      <div className="flex-1 min-h-0 overflow-y-auto px-4 sm:px-6 py-4 sm:py-5">
        {loading ? (
          <div className="flex flex-col items-center justify-center h-48 text-muted-foreground">
            <p className="text-sm">Loading questions...</p>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-48 text-muted-foreground gap-3">
            <Search className="size-8 opacity-30" />
            <p className="text-sm text-center">
              {questions.length > 0
                ? `No questions match your current filters (${questions.length} total questions available).`
                : "No questions in repository yet. Import a PDF to get started."}
            </p>
            {questions.length > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleClearFilters}
              >
                Clear All Filters
              </Button>
            )}
          </div>
        ) : (
          <ul
            className="grid gap-4 grid-cols-[repeat(auto-fill,minmax(320px,1fr))]"
            aria-label="Question cards"
          >
            {filtered.map((q) => (
              <li key={q.id} className="min-w-0" style={{ contentVisibility: "auto", containIntrinsicSize: "0 350px" }}>
                <QuestionCard
                  {...q}
                  isAdded={selectedQuestionIds.includes(q.id)}
                  onAddToWorksheet={handleAdd}
                  onDelete={handleDelete}
                  onUpdate={handleUpdate}
                />
              </li>
            ))}
          </ul>
        )}
      </div>

      <AddQuestionModal 
        open={showAddModal} 
        onOpenChange={setShowAddModal} 
        onSuccess={() => {
          setShowAddModal(false);
          fetchQuestions();
        }}
      />
      <ManagePapersModal
        open={showManagePapers}
        onOpenChange={setShowManagePapers}
        onPaperDeleted={fetchQuestions}
      />
    </section>
  );
}
