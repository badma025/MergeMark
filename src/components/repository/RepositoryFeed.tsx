import { Search, Plus, X, FileText } from "lucide-react";
import { useState, useEffect, useRef, useMemo, useDeferredValue, useCallback } from "react";
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

export interface RepositoryFeedProps {
  isActive?: boolean;
  onAddToWorksheet: (question: Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">) => void;
  onAddMultipleToWorksheet?: (questions: Omit<QuestionCardProps, "onAddToWorksheet" | "onDelete">[]) => void;
  selectedQuestionIds?: string[];
}

function QuestionCardSkeleton() {
  return (
    <div className="flex flex-col justify-between rounded-xl border border-border/50 bg-card/40 p-4 sm:p-5 shadow-xs animate-pulse min-h-[220px]">
      <div className="space-y-3.5">
        {/* Header Badges & Add Button */}
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-1.5 flex-wrap">
            <div className="h-5 w-20 rounded-md bg-muted/60" />
            <div className="h-5 w-24 rounded-md bg-muted/40" />
            <div className="h-5 w-14 rounded-md bg-muted/40" />
          </div>
          <div className="h-7 w-16 rounded-md bg-muted/50 shrink-0" />
        </div>

        {/* Content Skeleton Lines */}
        <div className="space-y-2 pt-1">
          <div className="h-3.5 w-full rounded bg-muted/40" />
          <div className="h-3.5 w-11/12 rounded bg-muted/30" />
          <div className="h-3.5 w-3/4 rounded bg-muted/20" />
        </div>
      </div>

      {/* Footer Badges & Actions */}
      <div className="flex items-center justify-between pt-3 border-t border-border/20 mt-4">
        <div className="h-4 w-28 rounded bg-muted/30" />
        <div className="flex items-center gap-1.5">
          <div className="size-6 rounded-md bg-muted/30" />
          <div className="size-6 rounded-md bg-muted/30" />
        </div>
      </div>
    </div>
  );
}

export function RepositoryFeed({
  isActive = true,
  onAddToWorksheet,
  onAddMultipleToWorksheet,
  selectedQuestionIds = [],
}: RepositoryFeedProps) {
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
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
  const splashDismissedRef = useRef(false);
  const { subjects, topicsBySubject, loading: taxonomyLoading } = useTaxonomy();

  const subjectNames = useMemo(() => (subjects || []).map(s => s?.name).filter(Boolean), [subjects]);

  const ALL_TOPICS = useMemo(() => {
    if (!topicsBySubject) return [];
    return Array.from(new Set(
      Object.values(topicsBySubject)
        .filter(Boolean)
        .flatMap(subjectMods => Object.values(subjectMods || {}).flat().filter(Boolean))
    ));
  }, [topicsBySubject]);

  const availablePapers = useMemo(() => {
    return Array.from(
      new Set(
        questions
          .map((q) => (q as any).paperName)
          .filter((name): name is string => typeof name === "string" && name.trim().length > 0)
      )
    ).sort();
  }, [questions]);

  // Pre-index questions to make search and multi-filtering virtually instantaneous
  const indexedQuestions = useMemo(() => {
    return questions.map((q) => {
      let parsedTopics: string[] = [];
      try {
        if (q.topics) {
          const parsed = JSON.parse(q.topics);
          if (Array.isArray(parsed)) parsedTopics = parsed;
        }
      } catch {}

      const resolvedSubject =
        subjects.find(
          (s) =>
            s.id === q.subject ||
            s.name.toLowerCase() === (q.subject || "").toLowerCase()
        )?.name ||
        q.subject ||
        "";

      const paper = ((q as any).paperName || "").trim();
      const mathSnippet = ((q as any).mathSnippet || "").trim();

      // Lowercase search corpus created once per question update
      const searchCorpus = `${q.subject || ""} ${q.subtopic || ""} ${q.content || ""} ${paper} ${mathSnippet} ${parsedTopics.join(" ")}`.toLowerCase();

      return {
        raw: q,
        searchCorpus,
        parsedTopics,
        resolvedSubject,
        paper,
      };
    });
  }, [questions, subjects]);

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

  // Dismiss splash screen ONLY when questions and taxonomy are fully ready and painted in the DOM
  useEffect(() => {
    if (!loading && !taxonomyLoading && !splashDismissedRef.current) {
      splashDismissedRef.current = true;
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          setTimeout(() => {
            (window as unknown as { __dismissSplash?: () => void }).__dismissSplash?.();
          }, 150);
        });
      });
    }
  }, [loading, taxonomyLoading, questions.length]);

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

  const handleDelete = useCallback(async (id: string) => {
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
  }, []);

  const handleUpdate = useCallback(async (id: string, newContent: string, newMarks: number, newAnswerContent?: string, newTopics?: string[], newModule?: string) => {
    try {
      const newTopicsStr = newTopics ? JSON.stringify(newTopics) : undefined;
      await invoke("update_question", { id, newContent, newMarks, newAnswerContent, newTopics: newTopicsStr, newModule });
      setQuestions((prev) => 
        prev.map(q => q.id === id ? { ...q, content: newContent, marks: newMarks, answerContent: newAnswerContent, topics: newTopicsStr ?? q.topics, module: newModule ?? (q as any).module } : q)
      );
      toast.success("Question updated successfully");
    } catch (err) {
      toast.error("Failed to update question", { description: String(err) });
    }
  }, []);

  const handleClearFilters = useCallback(() => {
    setSearch("");
    setSelectedSubject("All");
    setSelectedModule("All");
    setSelectedTopics([]);
    setSelectedPaper("All");
    setSelectedMarksRange("All");
    setReviewFilter("All");
  }, []);

  const hasActiveFilters =
    search !== "" ||
    selectedSubject !== "All" ||
    selectedModule !== "All" ||
    selectedTopics.length > 0 ||
    selectedPaper !== "All" ||
    selectedMarksRange !== "All" ||
    reviewFilter !== "All";

  const filtered = useMemo(() => {
    const term = deferredSearch.toLowerCase().trim();

    return indexedQuestions
      .filter(({ raw: q, searchCorpus, parsedTopics, resolvedSubject, paper }) => {
        // Instant search check against pre-indexed corpus
        if (term.length > 0 && !searchCorpus.includes(term)) {
          return false;
        }

        if (
          selectedSubject !== "All" &&
          resolvedSubject.toLowerCase() !== selectedSubject.toLowerCase()
        ) {
          return false;
        }

        if (
          selectedPaper !== "All" &&
          paper.toLowerCase() !== selectedPaper.toLowerCase()
        ) {
          return false;
        }

        if (selectedMarksRange === "1-2" && (q.marks < 1 || q.marks > 2)) return false;
        if (selectedMarksRange === "3-5" && (q.marks < 3 || q.marks > 5)) return false;
        if (selectedMarksRange === "6+" && q.marks < 6) return false;

        if (selectedTopics.length > 0 && !parsedTopics.some((t) => selectedTopics.includes(t))) {
          return false;
        }

        if (selectedModule !== "All") {
          const qMod = (q as any).module;
          if (qMod && qMod !== "Unknown" && qMod !== "General") {
            if (qMod !== selectedModule) return false;
          } else {
            const moduleTopics = (topicsBySubject[resolvedSubject] || {})[selectedModule] || [];
            if (selectedTopics.length === 0 && parsedTopics.length > 0) {
              if (!parsedTopics.some((t) => moduleTopics.includes(t))) return false;
            }
          }
        }

        if (reviewFilter === "Clean" && (q.needsReview || q.answerStale)) return false;
        if (reviewFilter === "Needs review" && !q.needsReview && !q.answerStale) return false;

        return true;
      })
      .map((item) => item.raw);
  }, [
    indexedQuestions,
    deferredSearch,
    selectedSubject,
    selectedPaper,
    selectedMarksRange,
    selectedTopics,
    selectedModule,
    reviewFilter,
    topicsBySubject,
  ]);

  const totalMarksFiltered = useMemo(() => {
    return filtered.reduce((sum, q) => sum + (q.marks || 0), 0);
  }, [filtered]);

  const INITIAL_DISPLAY_COUNT = 12;
  const [displayCount, setDisplayCount] = useState(INITIAL_DISPLAY_COUNT);

  // Expand displayCount smoothly after initial paint without blocking the UI
  useEffect(() => {
    if (!loading && filtered.length > displayCount) {
      const timer = setTimeout(() => {
        setDisplayCount(60);
      }, 400);
      return () => clearTimeout(timer);
    }
  }, [loading, filtered.length, displayCount]);

  useEffect(() => {
    setDisplayCount(INITIAL_DISPLAY_COUNT);
  }, [
    deferredSearch,
    selectedSubject,
    selectedPaper,
    selectedMarksRange,
    selectedTopics,
    selectedModule,
    reviewFilter,
  ]);

  const visibleQuestions = useMemo(() => {
    return filtered.slice(0, displayCount);
  }, [filtered, displayCount]);

  const handleAdd = useCallback((id: string) => {
    const question = questions.find((q) => q.id === id);
    if (question) {
      onAddToWorksheet(question);
    }
  }, [questions, onAddToWorksheet]);

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
              <div className="w-[180px] sm:w-[220px] md:w-[260px]">
                <Select
                  value={selectedPaper}
                  onValueChange={(v) => {
                    if (v) setSelectedPaper(v);
                  }}
                >
                  <SelectTrigger className="h-8 text-xs font-semibold bg-muted/40 w-full">
                    <FileText className="size-3.5 mr-1.5 text-muted-foreground shrink-0" />
                    <SelectValue placeholder="All Papers" />
                  </SelectTrigger>
                  <SelectContent className="min-w-[max(100%,320px)] max-w-[500px]">
                    <SelectItem value="All">All Papers ({availablePapers.length})</SelectItem>
                    {availablePapers.map((p) => (
                      <SelectItem key={p} value={p} title={p}>
                        {p}
                      </SelectItem>
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
          <div className="flex flex-col gap-6">
            <ul
              className="grid gap-4 grid-cols-[repeat(auto-fill,minmax(320px,1fr))]"
              aria-label="Loading question cards"
            >
              {Array.from({ length: 6 }).map((_, i) => (
                <li key={`skeleton-${i}`} className="min-w-0">
                  <QuestionCardSkeleton />
                </li>
              ))}
            </ul>
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
          <div className="flex flex-col gap-6">
            <ul
              className="grid gap-4 grid-cols-[repeat(auto-fill,minmax(320px,1fr))]"
              aria-label="Question cards"
            >
              {visibleQuestions.map((q) => (
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

            {filtered.length > displayCount && (
              <div className="flex justify-center pb-4 pt-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setDisplayCount((prev) => prev + 60)}
                  className="gap-2 text-xs font-semibold px-5 h-9 bg-card/60 hover:bg-card border-border/80 shadow-xs hover:border-primary/50 transition-all"
                >
                  <span>Show More Questions ({filtered.length - displayCount} remaining)</span>
                </Button>
              </div>
            )}
          </div>
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
