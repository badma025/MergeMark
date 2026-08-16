import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
  ChevronRight,
  ChevronDown,
  FolderTree,
  Plus,
  Trash2,
  Edit2,
  BrainCircuit,
  Search,
  BookOpen,
  Folder,
  Tag,
  Check,
  X
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export interface Topic {
  id: string;
  moduleId: string;
  name: string;
}

export interface Module {
  id: string;
  subjectId: string;
  name: string;
  topics: Topic[];
}

export interface Subject {
  id: string;
  name: string;
  modules: Module[];
}

export function TaxonomyManager() {
  const [subjects, setSubjects] = useState<Subject[]>([]);
  const [loading, setLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");

  const [expandedSubjects, setExpandedSubjects] = useState<Set<string>>(new Set());
  const [expandedModules, setExpandedModules] = useState<Set<string>>(new Set());

  // In-line editing states
  const [addingSubject, setAddingSubject] = useState(false);
  const [newSubjectName, setNewSubjectName] = useState("");

  const [addingModuleFor, setAddingModuleFor] = useState<string | null>(null);
  const [newModuleName, setNewModuleName] = useState("");

  const [addingTopicFor, setAddingTopicFor] = useState<string | null>(null);
  const [newTopicName, setNewTopicName] = useState("");

  const [editingItem, setEditingItem] = useState<{
    type: "subject" | "module" | "topic";
    id: string;
    name: string;
  } | null>(null);

  const [generatingFor, setGeneratingFor] = useState<string | null>(null);

  const loadTaxonomy = async () => {
    try {
      const data = await invoke<Subject[]>("get_taxonomy_tree");
      const list = Array.isArray(data) ? data : [];
      setSubjects(list);
      await emit("taxonomy-changed");
    } catch (err) {
      toast.error("Failed to load taxonomy", { description: String(err) });
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadTaxonomy();
  }, []);

  const toggleSubject = (id: string) => {
    const next = new Set(expandedSubjects);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpandedSubjects(next);
  };

  const toggleModule = (id: string) => {
    const next = new Set(expandedModules);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpandedModules(next);
  };

  const expandAll = () => {
    const allSubs = new Set(subjects.map(s => s.id));
    const allMods = new Set(subjects.flatMap(s => (s.modules || []).map(m => m.id)));
    setExpandedSubjects(allSubs);
    setExpandedModules(allMods);
  };

  const collapseAll = () => {
    setExpandedSubjects(new Set());
    setExpandedModules(new Set());
  };

  // ── Handlers ──

  const handleAddSubject = async () => {
    if (!newSubjectName.trim()) return;
    try {
      await invoke("add_subject", { name: newSubjectName.trim() });
      toast.success("Subject added");
      setNewSubjectName("");
      setAddingSubject(false);
      loadTaxonomy();
    } catch (err) {
      toast.error("Failed to add subject", { description: String(err) });
    }
  };

  const handleAddModule = async (subjectId: string) => {
    if (!newModuleName.trim()) return;
    try {
      await invoke("add_module", { subjectId, name: newModuleName.trim() });
      toast.success("Module added");
      setNewModuleName("");
      setAddingModuleFor(null);
      loadTaxonomy();
      setExpandedSubjects(prev => new Set(prev).add(subjectId));
    } catch (err) {
      toast.error("Failed to add module", { description: String(err) });
    }
  };

  const handleAddTopic = async (moduleId: string) => {
    if (!newTopicName.trim()) return;
    try {
      await invoke("add_topic", { moduleId, name: newTopicName.trim() });
      toast.success("Topic added");
      setNewTopicName("");
      setAddingTopicFor(null);
      loadTaxonomy();
      setExpandedModules(prev => new Set(prev).add(moduleId));
    } catch (err) {
      toast.error("Failed to add topic", { description: String(err) });
    }
  };

  const handleRename = async () => {
    if (!editingItem || !editingItem.name.trim()) return;
    try {
      if (editingItem.type === "subject") {
        await invoke("rename_subject", { id: editingItem.id, name: editingItem.name.trim() });
      } else if (editingItem.type === "module") {
        await invoke("rename_module", { id: editingItem.id, name: editingItem.name.trim() });
      } else if (editingItem.type === "topic") {
        await invoke("rename_topic", { id: editingItem.id, name: editingItem.name.trim() });
      }
      toast.success("Renamed successfully");
      setEditingItem(null);
      loadTaxonomy();
    } catch (err) {
      toast.error("Failed to rename item", { description: String(err) });
    }
  };

  const handleDeleteSubject = async (id: string) => {
    try {
      await invoke("delete_subject", { id });
      toast.success("Subject deleted");
      loadTaxonomy();
    } catch (err) {
      toast.error("Failed to delete subject", { description: String(err) });
    }
  };

  const handleDeleteModule = async (id: string) => {
    try {
      await invoke("delete_module", { id });
      toast.success("Module deleted");
      loadTaxonomy();
    } catch (err) {
      toast.error("Failed to delete module", { description: String(err) });
    }
  };

  const handleDeleteTopic = async (id: string) => {
    try {
      await invoke("delete_topic", { id });
      toast.success("Topic deleted");
      loadTaxonomy();
    } catch (err) {
      toast.error("Failed to delete topic", { description: String(err) });
    }
  };

  const handleGenerateTopics = async (moduleId: string, moduleName: string, subjectName: string) => {
    setGeneratingFor(moduleId);
    const modelName = localStorage.getItem("mergemark_openai_model") || "google/gemini-2.5-flash";
    try {
      await invoke("generate_topics_for_module", {
        subjectName,
        moduleName,
        moduleId,
        modelName
      });
      toast.success(`Generated topics for ${moduleName}!`);
      loadTaxonomy();
      setExpandedModules(prev => new Set(prev).add(moduleId));
    } catch (e) {
      toast.error("Failed to generate topics", { description: String(e) });
    } finally {
      setGeneratingFor(null);
    }
  };

  // Filtered tree by search query
  const filteredSubjects = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return subjects;

    return subjects.map(s => {
      const subjectMatches = s.name.toLowerCase().includes(q);
      const filteredModules = (s.modules || []).map(m => {
        const moduleMatches = m.name.toLowerCase().includes(q);
        const filteredTopics = (m.topics || []).filter(t => t.name.toLowerCase().includes(q));
        if (subjectMatches || moduleMatches || filteredTopics.length > 0) {
          return { ...m, topics: filteredTopics.length > 0 ? filteredTopics : m.topics };
        }
        return null;
      }).filter(Boolean) as Module[];

      if (subjectMatches || filteredModules.length > 0) {
        return { ...s, modules: filteredModules };
      }
      return null;
    }).filter(Boolean) as Subject[];
  }, [subjects, searchQuery]);

  const totalSubjects = subjects.length;
  const totalModules = subjects.reduce((acc, s) => acc + (s.modules?.length || 0), 0);
  const totalTopics = subjects.reduce((acc, s) => acc + (s.modules?.reduce((mAcc, m) => mAcc + (m.topics?.length || 0), 0) || 0), 0);

  if (loading) {
    return (
      <div className="p-12 text-center text-xs text-muted-foreground flex flex-col items-center justify-center gap-2">
        <FolderTree className="size-6 animate-pulse text-primary opacity-60" />
        <span>Loading curriculum taxonomy...</span>
      </div>
    );
  }

  return (
    <div className="w-full flex flex-col gap-5">
      {/* ── Taxonomy Header & Controls ── */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 pb-4">
        <div>
          <h2 className="text-base font-bold flex items-center gap-2 text-foreground">
            <FolderTree className="size-4.5 text-primary" />
            Curriculum Taxonomy Explorer
          </h2>
          <p className="text-xs text-muted-foreground mt-0.5">
            Manage the subjects, examination modules, and syllabus topics used to tag and filter questions.
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={expandedSubjects.size > 0 ? collapseAll : expandAll}
            className="h-8 text-xs font-medium"
          >
            {expandedSubjects.size > 0 ? "Collapse All" : "Expand All"}
          </Button>

          <Button
            variant="default"
            size="sm"
            onClick={() => setAddingSubject(true)}
            className="h-8 text-xs font-semibold gap-1.5 shadow-xs"
          >
            <Plus className="size-3.5" />
            <span>Add Subject</span>
          </Button>
        </div>
      </div>

      {/* ── Search & Metrics Bar ── */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="relative w-full max-w-sm">
          <Search className="size-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search subjects, modules, or topics..."
            className="h-8 pl-8 text-xs font-sans bg-muted/30"
          />
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery("")}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          )}
        </div>

        <div className="flex items-center gap-2 text-xs font-mono text-muted-foreground">
          <Badge variant="outline" className="font-normal gap-1 bg-muted/40">
            <BookOpen className="size-3 text-primary" />
            <span>{totalSubjects} Subjects</span>
          </Badge>
          <Badge variant="outline" className="font-normal gap-1 bg-muted/40">
            <Folder className="size-3 text-blue-500" />
            <span>{totalModules} Modules</span>
          </Badge>
          <Badge variant="outline" className="font-normal gap-1 bg-muted/40">
            <Tag className="size-3 text-emerald-500" />
            <span>{totalTopics} Topics</span>
          </Badge>
        </div>
      </div>

      {/* ── New Subject In-line Creation ── */}
      {addingSubject && (
        <div className="flex items-center gap-2.5 p-3.5 rounded-xl bg-primary/5 border border-primary/20 animate-in fade-in-50 duration-150">
          <BookOpen className="size-4 text-primary shrink-0" />
          <Input
            value={newSubjectName}
            onChange={(e) => setNewSubjectName(e.target.value)}
            placeholder="Enter new subject name (e.g. A Level Chemistry (AQA))..."
            className="h-8 text-xs font-medium bg-background"
            onKeyDown={(e) => e.key === "Enter" && handleAddSubject()}
            autoFocus
          />
          <Button size="sm" variant="default" className="h-8 text-xs font-semibold px-3" onClick={handleAddSubject}>
            <Check className="size-3.5 mr-1" /> Save
          </Button>
          <Button size="sm" variant="ghost" className="h-8 text-xs px-2.5 text-muted-foreground" onClick={() => setAddingSubject(false)}>
            Cancel
          </Button>
        </div>
      )}

      {/* ── Hierarchical Taxonomy Tree ── */}
      <div className="flex flex-col gap-2 rounded-xl border border-border/60 bg-muted/10 p-2 sm:p-3">
        {filteredSubjects.length === 0 ? (
          <div className="p-8 text-center text-xs text-muted-foreground flex flex-col items-center justify-center gap-1.5">
            <FolderTree className="size-6 opacity-30" />
            <span>{searchQuery ? "No matching curriculum items found." : "No subjects defined yet."}</span>
            <span className="text-[11px] opacity-75">Click 'Add Subject' above to create your first curriculum branch.</span>
          </div>
        ) : (
          filteredSubjects.map((subject) => {
            const isSubExpanded = expandedSubjects.has(subject.id) || !!searchQuery;
            const moduleCount = subject.modules?.length || 0;

            return (
              <div
                key={subject.id}
                className="flex flex-col rounded-lg border border-border/50 bg-card/60 overflow-hidden transition-all"
              >
                {/* ── Subject Row ── */}
                <div
                  className={cn(
                    "flex items-center justify-between p-2.5 px-3 hover:bg-muted/40 cursor-pointer transition-colors group select-none",
                    isSubExpanded && "bg-muted/20 border-b border-border/40"
                  )}
                  onClick={() => toggleSubject(subject.id)}
                >
                  <div className="flex items-center gap-2.5 flex-1 min-w-0">
                    <button
                      type="button"
                      className="size-5 rounded flex items-center justify-center text-muted-foreground hover:text-foreground"
                    >
                      {isSubExpanded ? (
                        <ChevronDown className="size-4 text-primary" />
                      ) : (
                        <ChevronRight className="size-4" />
                      )}
                    </button>

                    <BookOpen className="size-4 text-primary shrink-0 opacity-80" />

                    {editingItem?.id === subject.id ? (
                      <div className="flex items-center gap-1.5 flex-1 max-w-md" onClick={(e) => e.stopPropagation()}>
                        <Input
                          autoFocus
                          value={editingItem.name}
                          onChange={(e) => setEditingItem({ ...editingItem, name: e.target.value })}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") handleRename();
                            if (e.key === "Escape") setEditingItem(null);
                          }}
                          className="h-7 text-xs font-semibold"
                        />
                        <Button size="sm" variant="ghost" className="h-7 px-2 text-emerald-500" onClick={handleRename}>
                          <Check className="size-3.5" />
                        </Button>
                        <Button size="sm" variant="ghost" className="h-7 px-2 text-muted-foreground" onClick={() => setEditingItem(null)}>
                          <X className="size-3.5" />
                        </Button>
                      </div>
                    ) : (
                      <span className="font-semibold text-xs text-foreground truncate">{subject.name}</span>
                    )}

                    <Badge variant="secondary" className="text-[10px] font-mono px-1.5 py-0 h-4.5 text-muted-foreground">
                      {moduleCount} {moduleCount === 1 ? "module" : "modules"}
                    </Badge>
                  </div>

                  {/* Actions */}
                  <div
                    className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-xs text-primary hover:bg-primary/10"
                      onClick={() => {
                        setAddingModuleFor(subject.id);
                        setExpandedSubjects(prev => new Set(prev).add(subject.id));
                      }}
                      title="Add Module"
                    >
                      <Plus className="size-3 mr-1" />
                      <span>Module</span>
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 text-muted-foreground hover:text-foreground"
                      onClick={() => setEditingItem({ type: "subject", id: subject.id, name: subject.name })}
                      title="Rename"
                    >
                      <Edit2 className="size-3" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 text-destructive hover:bg-destructive/10"
                      onClick={() => handleDeleteSubject(subject.id)}
                      title="Delete Subject"
                    >
                      <Trash2 className="size-3" />
                    </Button>
                  </div>
                </div>

                {/* ── Modules Container ── */}
                {isSubExpanded && (
                  <div className="flex flex-col p-2 pl-4 sm:pl-7 gap-2 bg-muted/10">
                    {/* Add Module Input */}
                    {addingModuleFor === subject.id && (
                      <div className="flex items-center gap-2 p-2 rounded-lg bg-background border border-border/80 animate-in fade-in-50">
                        <Folder className="size-3.5 text-blue-500 shrink-0" />
                        <Input
                          value={newModuleName}
                          onChange={(e) => setNewModuleName(e.target.value)}
                          placeholder="Module name (e.g. Pure Mathematics 1, Mechanics)..."
                          className="h-7 text-xs"
                          onKeyDown={(e) => e.key === "Enter" && handleAddModule(subject.id)}
                          autoFocus
                        />
                        <Button size="sm" variant="default" className="h-7 text-xs px-2.5" onClick={() => handleAddModule(subject.id)}>
                          Save
                        </Button>
                        <Button size="sm" variant="ghost" className="h-7 text-xs px-2 text-muted-foreground" onClick={() => setAddingModuleFor(null)}>
                          Cancel
                        </Button>
                      </div>
                    )}

                    {(!subject.modules || subject.modules.length === 0) && addingModuleFor !== subject.id && (
                      <p className="text-[11px] text-muted-foreground italic py-1 pl-2">
                        No modules in this subject yet. Click "+ Module" to add one.
                      </p>
                    )}

                    {subject.modules?.map((module) => {
                      const isModExpanded = expandedModules.has(module.id) || !!searchQuery;
                      const topicCount = module.topics?.length || 0;
                      const isGenerating = generatingFor === module.id;

                      return (
                        <div
                          key={module.id}
                          className="flex flex-col rounded-lg border border-border/40 bg-card/90 overflow-hidden"
                        >
                          {/* Module Header */}
                          <div
                            className={cn(
                              "flex items-center justify-between p-2 px-3 hover:bg-muted/30 cursor-pointer transition-colors group select-none",
                              isModExpanded && "bg-muted/15 border-b border-border/30"
                            )}
                            onClick={() => toggleModule(module.id)}
                          >
                            <div className="flex items-center gap-2 flex-1 min-w-0">
                              <button
                                type="button"
                                className="size-4 rounded flex items-center justify-center text-muted-foreground hover:text-foreground"
                              >
                                {isModExpanded ? (
                                  <ChevronDown className="size-3.5 text-blue-500" />
                                ) : (
                                  <ChevronRight className="size-3.5" />
                                )}
                              </button>

                              <Folder className="size-3.5 text-blue-500 shrink-0" />

                              {editingItem?.id === module.id ? (
                                <div className="flex items-center gap-1 flex-1 max-w-sm" onClick={(e) => e.stopPropagation()}>
                                  <Input
                                    autoFocus
                                    value={editingItem.name}
                                    onChange={(e) => setEditingItem({ ...editingItem, name: e.target.value })}
                                    onKeyDown={(e) => {
                                      if (e.key === "Enter") handleRename();
                                      if (e.key === "Escape") setEditingItem(null);
                                    }}
                                    className="h-6 text-xs"
                                  />
                                  <Button size="sm" variant="ghost" className="h-6 px-1.5 text-emerald-500" onClick={handleRename}>
                                    <Check className="size-3" />
                                  </Button>
                                  <Button size="sm" variant="ghost" className="h-6 px-1.5 text-muted-foreground" onClick={() => setEditingItem(null)}>
                                    <X className="size-3" />
                                  </Button>
                                </div>
                              ) : (
                                <span className="font-medium text-xs text-foreground truncate">{module.name}</span>
                              )}

                              <Badge variant="outline" className="text-[9px] font-mono px-1 py-0 h-4 text-muted-foreground">
                                {topicCount} {topicCount === 1 ? "topic" : "topics"}
                              </Badge>
                            </div>

                            {/* Module Actions */}
                            <div
                              className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity"
                              onClick={(e) => e.stopPropagation()}
                            >
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-1.5 text-[11px] text-amber-500 hover:bg-amber-500/10 gap-1"
                                onClick={() => handleGenerateTopics(module.id, module.name, subject.name)}
                                disabled={isGenerating}
                                title="Auto-generate core topics from official syllabus using AI"
                              >
                                <BrainCircuit className={cn("size-3", isGenerating && "animate-spin text-amber-500")} />
                                <span>{isGenerating ? "Generating..." : "Auto-Topics"}</span>
                              </Button>

                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-1.5 text-[11px] text-primary hover:bg-primary/10"
                                onClick={() => {
                                  setAddingTopicFor(module.id);
                                  setExpandedModules(prev => new Set(prev).add(module.id));
                                }}
                                title="Add Topic"
                              >
                                <Plus className="size-3 mr-0.5" />
                                <span>Topic</span>
                              </Button>

                              <Button
                                variant="ghost"
                                size="icon"
                                className="size-6 text-muted-foreground hover:text-foreground"
                                onClick={() => setEditingItem({ type: "module", id: module.id, name: module.name })}
                                title="Rename"
                              >
                                <Edit2 className="size-2.5" />
                              </Button>

                              <Button
                                variant="ghost"
                                size="icon"
                                className="size-6 text-destructive hover:bg-destructive/10"
                                onClick={() => handleDeleteModule(module.id)}
                                title="Delete Module"
                              >
                                <Trash2 className="size-2.5" />
                              </Button>
                            </div>
                          </div>

                          {/* Topics List */}
                          {isModExpanded && (
                            <div className="flex flex-col p-2.5 pl-6 gap-2 bg-muted/5">
                              {/* Add Topic Input */}
                              {addingTopicFor === module.id && (
                                <div className="flex items-center gap-2 p-1.5 rounded-lg bg-background border border-border/80 animate-in fade-in-50">
                                  <Tag className="size-3 text-emerald-500 shrink-0" />
                                  <Input
                                    value={newTopicName}
                                    onChange={(e) => setNewTopicName(e.target.value)}
                                    placeholder="Topic name (e.g. Differentiation, Complex Numbers)..."
                                    className="h-6 text-xs"
                                    onKeyDown={(e) => e.key === "Enter" && handleAddTopic(module.id)}
                                    autoFocus
                                  />
                                  <Button size="sm" variant="default" className="h-6 text-xs px-2" onClick={() => handleAddTopic(module.id)}>
                                    Save
                                  </Button>
                                  <Button size="sm" variant="ghost" className="h-6 text-xs px-1.5 text-muted-foreground" onClick={() => setAddingTopicFor(null)}>
                                    Cancel
                                  </Button>
                                </div>
                              )}

                              {(!module.topics || module.topics.length === 0) && addingTopicFor !== module.id ? (
                                <p className="text-[11px] text-muted-foreground italic py-1">
                                  No topics in this module. Click "+ Topic" or "Auto-Topics" to populate.
                                </p>
                              ) : (
                                <div className="flex flex-wrap gap-1.5">
                                  {module.topics?.map((topic) => (
                                    <div
                                      key={topic.id}
                                      className="group/topic inline-flex items-center gap-1 px-2.5 py-1 rounded-md bg-muted/40 hover:bg-muted/70 border border-border/60 text-xs text-foreground transition-all"
                                    >
                                      {editingItem?.id === topic.id ? (
                                        <div className="flex items-center gap-1">
                                          <input
                                            autoFocus
                                            value={editingItem.name}
                                            onChange={(e) => setEditingItem({ ...editingItem, name: e.target.value })}
                                            onKeyDown={(e) => {
                                              if (e.key === "Enter") handleRename();
                                              if (e.key === "Escape") setEditingItem(null);
                                            }}
                                            className="h-5 px-1 bg-background text-xs rounded border border-border w-28"
                                          />
                                          <button type="button" onClick={handleRename} className="text-emerald-500 hover:text-emerald-400">
                                            <Check className="size-3" />
                                          </button>
                                          <button type="button" onClick={() => setEditingItem(null)} className="text-muted-foreground">
                                            <X className="size-3" />
                                          </button>
                                        </div>
                                      ) : (
                                        <>
                                          <Tag className="size-2.5 text-emerald-500 opacity-80" />
                                          <span className="truncate max-w-[200px]">{topic.name}</span>
                                          <div className="flex items-center opacity-0 group-hover/topic:opacity-100 transition-opacity ml-1 gap-0.5">
                                            <button
                                              type="button"
                                              onClick={() => setEditingItem({ type: "topic", id: topic.id, name: topic.name })}
                                              className="p-0.5 text-muted-foreground hover:text-foreground rounded"
                                              title="Rename topic"
                                            >
                                              <Edit2 className="size-2.5" />
                                            </button>
                                            <button
                                              type="button"
                                              onClick={() => handleDeleteTopic(topic.id)}
                                              className="p-0.5 text-muted-foreground hover:text-destructive rounded"
                                              title="Delete topic"
                                            >
                                              <Trash2 className="size-2.5" />
                                            </button>
                                          </div>
                                        </>
                                      )}
                                    </div>
                                  ))}
                                </div>
                              )}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
