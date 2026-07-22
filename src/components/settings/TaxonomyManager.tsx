import { useState, useEffect } from "react";
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
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

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
      setSubjects(data);
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

  // ── Handlers ──

  const handleAddSubject = async () => {
    if (!newSubjectName.trim()) return;
    try {
      await invoke("add_subject", { name: newSubjectName.trim() });
      toast.success("Subject added");
      setNewSubjectName("");
      setAddingSubject(false);
      loadTaxonomy();
    } catch (e) {
      toast.error("Failed to add subject", { description: String(e) });
    }
  };

  const handleDeleteSubject = async (id: string) => {
    if (!confirm("Are you sure? This will delete all modules and topics within this subject.")) return;
    try {
      await invoke("delete_subject", { id });
      toast.success("Subject deleted");
      loadTaxonomy();
    } catch (e) {
      toast.error("Failed to delete subject", { description: String(e) });
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
    } catch (e) {
      toast.error("Failed to add module", { description: String(e) });
    }
  };

  const handleDeleteModule = async (id: string) => {
    if (!confirm("Are you sure? This will delete all topics within this module.")) return;
    try {
      await invoke("delete_module", { id });
      toast.success("Module deleted");
      loadTaxonomy();
    } catch (e) {
      toast.error("Failed to delete module", { description: String(e) });
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
    } catch (e) {
      toast.error("Failed to add topic", { description: String(e) });
    }
  };

  const handleDeleteTopic = async (id: string) => {
    if (!confirm("Are you sure you want to delete this topic?")) return;
    try {
      await invoke("delete_topic", { id });
      toast.success("Topic deleted");
      loadTaxonomy();
    } catch (e) {
      toast.error("Failed to delete topic", { description: String(e) });
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
    } catch (e) {
      toast.error("Failed to rename", { description: String(e) });
    }
  };

  const handleGenerateTopics = async (moduleId: string) => {
    setGeneratingFor(moduleId);
    try {
      let apiKey = localStorage.getItem("mergemark_openai_key") || "";
      if (!apiKey || apiKey.trim() === "") {
        apiKey = "dummy";
      }
      const baseUrl = localStorage.getItem("mergemark_openai_base_url") || "https://openrouter.ai/api/v1/";
      const modelName = localStorage.getItem("mergemark_openai_model") || "google/gemini-2.5-flash";

      // Need to invoke LLM to generate topics
      await invoke("generate_topics_for_module", { 
        moduleId,
        apiKey,
        baseUrl,
        modelName
      });
      toast.success("Topics generated!");
      loadTaxonomy();
      setExpandedModules(prev => new Set(prev).add(moduleId));
    } catch (e) {
      toast.error("Failed to generate topics", { description: String(e) });
    } finally {
      setGeneratingFor(null);
    }
  };

  if (loading) {
    return <div className="p-6 text-center text-sm text-muted-foreground">Loading curriculum...</div>;
  }

  return (
    <div className="w-full max-w-md flex flex-col gap-4 rounded-2xl border border-border/60 bg-card p-6 shadow-sm mb-12">
      <div className="flex items-center justify-between mb-2">
        <h2 className="text-sm font-bold flex items-center gap-2 text-foreground">
          <FolderTree className="size-4 text-primary" />
          Curriculum Taxonomy
        </h2>
        <Button variant="outline" size="sm" className="h-7 text-xs" onClick={() => setAddingSubject(true)}>
          <Plus className="size-3 mr-1" /> Add Subject
        </Button>
      </div>
      <p className="text-sm text-muted-foreground mb-2">
        Manage the subjects, modules, and topics used to categorize your questions.
      </p>

      {addingSubject && (
        <div className="flex items-center gap-2 mb-4 bg-muted/30 p-2 rounded-lg">
          <Input
            value={newSubjectName}
            onChange={(e) => setNewSubjectName(e.target.value)}
            placeholder="Subject name..."
            className="h-8 text-sm"
            onKeyDown={(e) => e.key === "Enter" && handleAddSubject()}
            autoFocus
          />
          <Button size="sm" variant="default" className="h-8" onClick={handleAddSubject}>Save</Button>
          <Button size="sm" variant="ghost" className="h-8" onClick={() => setAddingSubject(false)}>Cancel</Button>
        </div>
      )}

      <div className="flex flex-col gap-1 border-t border-border/40 pt-2">
        {subjects.length === 0 && !addingSubject && (
          <p className="text-xs text-muted-foreground italic p-2">No subjects defined yet.</p>
        )}
        
        {subjects.map(subject => (
          <div key={subject.id} className="flex flex-col select-none">
            {/* Subject Row */}
            <div className="flex items-center justify-between group hover:bg-muted/40 p-1.5 rounded-md cursor-pointer" onClick={() => toggleSubject(subject.id)}>
              <div className="flex items-center gap-2 flex-1">
                {expandedSubjects.has(subject.id) ? <ChevronDown className="size-4 text-muted-foreground" /> : <ChevronRight className="size-4 text-muted-foreground" />}
                {editingItem?.id === subject.id ? (
                  <textarea
                    autoFocus
                    value={editingItem.name}
                    onChange={(e) => setEditingItem({ ...editingItem, name: e.target.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        handleRename();
                      }
                      if (e.key === "Escape") setEditingItem(null);
                    }}
                    onBlur={handleRename}
                    onClick={e => e.stopPropagation()}
                    className="flex min-h-[40px] flex-1 rounded-md border border-input bg-transparent px-3 py-1.5 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                ) : (
                  <span className="font-semibold text-sm">{subject.name}</span>
                )}
              </div>
              
              <div className="flex items-center opacity-0 group-hover:opacity-100 transition-opacity gap-1" onClick={e => e.stopPropagation()}>
                {editingItem?.id === subject.id ? (
                  <>
                    <Button variant="ghost" size="icon" className="h-6 w-6" onClick={handleRename}><Plus className="size-3" /></Button>
                    <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => setEditingItem(null)}><Trash2 className="size-3 text-destructive" /></Button>
                  </>
                ) : (
                  <>
                    <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => setAddingModuleFor(subject.id)} title="Add Module"><Plus className="size-3" /></Button>
                    <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => setEditingItem({ type: "subject", id: subject.id, name: subject.name })} title="Rename"><Edit2 className="size-3" /></Button>
                    <Button variant="ghost" size="icon" className="h-6 w-6 text-destructive" onClick={() => handleDeleteSubject(subject.id)} title="Delete"><Trash2 className="size-3" /></Button>
                  </>
                )}
              </div>
            </div>

            {/* Modules */}
            {expandedSubjects.has(subject.id) && (
              <div className="flex flex-col ml-6 pl-2 border-l border-border/40 my-1 gap-1">
                {addingModuleFor === subject.id && (
                  <div className="flex items-center gap-2 mb-2 p-1">
                    <Input
                      value={newModuleName}
                      onChange={(e) => setNewModuleName(e.target.value)}
                      placeholder="Module name..."
                      className="h-7 text-xs"
                      onKeyDown={(e) => e.key === "Enter" && handleAddModule(subject.id)}
                      autoFocus
                    />
                    <Button size="sm" variant="default" className="h-7 text-xs" onClick={() => handleAddModule(subject.id)}>Save</Button>
                    <Button size="sm" variant="ghost" className="h-7 text-xs" onClick={() => setAddingModuleFor(null)}>Cancel</Button>
                  </div>
                )}
                
                {subject.modules.length === 0 && addingModuleFor !== subject.id && (
                  <p className="text-xs text-muted-foreground italic p-1">No modules.</p>
                )}

                {subject.modules.map(mod => (
                  <div key={mod.id} className="flex flex-col">
                    {/* Module Row */}
                    <div className="flex items-center justify-between group hover:bg-muted/40 p-1 rounded-md cursor-pointer" onClick={() => toggleModule(mod.id)}>
                      <div className="flex items-center gap-2 flex-1">
                        {expandedModules.has(mod.id) ? <ChevronDown className="size-3 text-muted-foreground" /> : <ChevronRight className="size-3 text-muted-foreground" />}
                        {editingItem?.id === mod.id ? (
                          <textarea
                            autoFocus
                            value={editingItem.name}
                            onChange={(e) => setEditingItem({ ...editingItem, name: e.target.value })}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" && !e.shiftKey) {
                                e.preventDefault();
                                handleRename();
                              }
                              if (e.key === "Escape") setEditingItem(null);
                            }}
                            onBlur={handleRename}
                            onClick={e => e.stopPropagation()}
                            className="flex min-h-[40px] flex-1 rounded-md border border-input bg-transparent px-3 py-1.5 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                          />
                        ) : (
                          <span className="font-medium text-sm text-foreground/90">{mod.name}</span>
                        )}
                      </div>

                      <div className="flex items-center opacity-0 group-hover:opacity-100 transition-opacity gap-1" onClick={e => e.stopPropagation()}>
                        {editingItem?.id === mod.id ? (
                          <>
                            <Button variant="ghost" size="icon" className="h-5 w-5" onClick={handleRename}><Plus className="size-3" /></Button>
                            <Button variant="ghost" size="icon" className="h-5 w-5" onClick={() => setEditingItem(null)}><Trash2 className="size-3 text-destructive" /></Button>
                          </>
                        ) : (
                          <>
                            <Button variant="ghost" size="icon" className="h-5 w-5" onClick={() => handleGenerateTopics(mod.id)} title="AI Generate Topics" disabled={generatingFor === mod.id}>
                              {generatingFor === mod.id ? <BrainCircuit className="size-3 animate-pulse text-primary" /> : <BrainCircuit className="size-3 text-primary" />}
                            </Button>
                            <Button variant="ghost" size="icon" className="h-5 w-5" onClick={() => setAddingTopicFor(mod.id)} title="Add Topic"><Plus className="size-3" /></Button>
                            <Button variant="ghost" size="icon" className="h-5 w-5" onClick={() => setEditingItem({ type: "module", id: mod.id, name: mod.name })} title="Rename"><Edit2 className="size-3" /></Button>
                            <Button variant="ghost" size="icon" className="h-5 w-5 text-destructive" onClick={() => handleDeleteModule(mod.id)} title="Delete"><Trash2 className="size-3" /></Button>
                          </>
                        )}
                      </div>
                    </div>

                    {/* Topics */}
                    {expandedModules.has(mod.id) && (
                      <div className="flex flex-col ml-6 pl-2 border-l border-border/40 my-1 gap-1">
                        {addingTopicFor === mod.id && (
                          <div className="flex items-center gap-2 mb-1 p-1">
                            <Input
                              value={newTopicName}
                              onChange={(e) => setNewTopicName(e.target.value)}
                              placeholder="Topic name..."
                              className="h-6 text-xs"
                              onKeyDown={(e) => e.key === "Enter" && handleAddTopic(mod.id)}
                              autoFocus
                            />
                            <Button size="sm" variant="default" className="h-6 text-[10px]" onClick={() => handleAddTopic(mod.id)}>Save</Button>
                            <Button size="sm" variant="ghost" className="h-6 text-[10px]" onClick={() => setAddingTopicFor(null)}>Cancel</Button>
                          </div>
                        )}
                        
                        {mod.topics.length === 0 && addingTopicFor !== mod.id && (
                          <p className="text-[10px] text-muted-foreground italic p-1">No topics.</p>
                        )}

                        {mod.topics.map(topic => (
                          <div key={topic.id} className="flex items-center justify-between group hover:bg-muted/40 p-1 rounded-md">
                            {editingItem?.id === topic.id ? (
                              <textarea
                                autoFocus
                                value={editingItem.name}
                                onChange={(e) => setEditingItem({ ...editingItem, name: e.target.value })}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter" && !e.shiftKey) {
                                    e.preventDefault();
                                    handleRename();
                                  }
                                  if (e.key === "Escape") setEditingItem(null);
                                }}
                                onBlur={handleRename}
                                className="flex min-h-[60px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                              />
                            ) : (
                              <span className="text-xs text-muted-foreground/90 pl-1">{topic.name}</span>
                            )}
                            
                            <div className="flex items-center opacity-0 group-hover:opacity-100 transition-opacity gap-1">
                              {editingItem?.id === topic.id ? (
                                <>
                                  <Button variant="ghost" size="icon" className="h-4 w-4" onClick={handleRename}><Plus className="size-3" /></Button>
                                  <Button variant="ghost" size="icon" className="h-4 w-4" onClick={() => setEditingItem(null)}><Trash2 className="size-3 text-destructive" /></Button>
                                </>
                              ) : (
                                <>
                                  <Button variant="ghost" size="icon" className="h-4 w-4" onClick={() => setEditingItem({ type: "topic", id: topic.id, name: topic.name })} title="Rename"><Edit2 className="size-[10px]" /></Button>
                                  <Button variant="ghost" size="icon" className="h-4 w-4 text-destructive" onClick={() => handleDeleteTopic(topic.id)} title="Delete"><Trash2 className="size-[10px]" /></Button>
                                </>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
