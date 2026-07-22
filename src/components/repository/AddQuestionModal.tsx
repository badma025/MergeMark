import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { RichTextEditor } from "./RichTextEditor";
import { useTaxonomy } from "@/lib/TaxonomyContext";
import { Plus } from "lucide-react";
import { cn } from "@/lib/utils";

export interface AddQuestionModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess: () => void;
}

export function AddQuestionModal({ open, onOpenChange, onSuccess }: AddQuestionModalProps) {
  const [subject, setSubject] = useState<string>("Mathematics");
  const [module, setModule] = useState<string>("");
  const [marks, setMarks] = useState<number>(1);
  const [topics, setTopics] = useState<string[]>([]);
  const [content, setContent] = useState<string>("");
  const [answerContent, setAnswerContent] = useState<string>("");
  const [isSubmitting, setIsSubmitting] = useState<boolean>(false);
  const { subjects, topicsBySubject } = useTaxonomy();
  const subjectNames = subjects.map(s => s.name);

  // Set default module when modal opens or subject changes
  useEffect(() => {
    if (open) {
      const subjectMods = topicsBySubject[subject] || {};
      const firstMod = Object.keys(subjectMods)[0] || "";
      if (!module) {
        setModule(firstMod);
      }
    }
  }, [open, subject]);

  // When subject changes, reset module and topics
  function handleSubjectChange(newSubject: string) {
    setSubject(newSubject);
    const subjectMods = topicsBySubject[newSubject] || {};
    const firstMod = Object.keys(subjectMods)[0] || "";
    setModule(firstMod);
    setTopics([]);
  }

  // When module changes, reset topics
  function handleModuleChange(newModule: string) {
    setModule(newModule);
    setTopics([]);
  }

  async function handleSave() {
    if (!content.trim()) {
      toast.error("Question content is required");
      return;
    }
    if (marks < 1) {
      toast.error("Marks must be at least 1");
      return;
    }

    setIsSubmitting(true);
    try {
      const newQuestion = {
        id: crypto.randomUUID(),
        subject,
        subtopic: module || "Manual entry",
        marks,
        content,
        mathSnippet: "",
        isCode: false,
        answerContent: answerContent || undefined,
        topics: JSON.stringify(topics),
        paperName: "",
        questionNumber: null,
        module: module || undefined,
      };

      await invoke("add_question", { question: newQuestion });
      toast.success("Question added manually");
      onSuccess();
      onOpenChange(false);
      
      // Reset form
      setSubject("Mathematics");
      setModule("");
      setMarks(1);
      setTopics([]);
      setContent("");
      setAnswerContent("");
    } catch (err) {
      toast.error("Failed to add question", { description: String(err) });
    } finally {
      setIsSubmitting(false);
    }
  }

  const subjectMods = topicsBySubject[subject] || {};
  const availableTopics = module ? subjectMods[module] || [] : Object.values(subjectMods).flat();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[95vw] sm:max-w-[95vw] h-[95vh] w-full flex flex-col p-6">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Plus className="size-5" />
            Add Question Manually
          </DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-4 py-2 flex-1 min-h-0 overflow-y-auto pr-2">
          {/* Top Controls Row */}
          <div className="flex items-center gap-4 flex-wrap bg-muted/30 p-3 rounded-lg border border-border/50">
            <div className="flex items-center gap-2">
              <label className="text-sm font-semibold text-foreground whitespace-nowrap">Subject:</label>
              <select
                value={subject}
                onChange={(e) => handleSubjectChange(e.target.value)}
                className="p-1.5 text-sm bg-background border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-primary/50"
              >
                {subjectNames.map((s) => (
                  <option key={s} value={s}>{s}</option>
                ))}
              </select>
            </div>

            {Object.keys(subjectMods).length > 0 && (
              <div className="flex items-center gap-2">
                <label className="text-sm font-semibold text-foreground whitespace-nowrap">Module:</label>
                <select
                  value={module}
                  onChange={(e) => handleModuleChange(e.target.value)}
                  className="p-1.5 text-sm bg-background border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-primary/50 max-w-[200px]"
                >
                  <option value="">Unknown / None</option>
                  {Object.keys(subjectMods).map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              </div>
            )}

            <div className="flex items-center gap-2">
              <label className="text-sm font-semibold text-foreground whitespace-nowrap">Marks:</label>
              <input
                type="number"
                min={1}
                max={100}
                value={marks}
                onChange={(e) => setMarks(parseInt(e.target.value) || 1)}
                className="w-20 p-1.5 text-sm bg-background border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-primary/50"
              />
            </div>
          </div>

          {/* Topics selection */}
          <div className="flex flex-col gap-2">
            <label className="text-sm font-semibold text-foreground">Topics:</label>
            <div className="flex flex-wrap items-center gap-1.5">
              {availableTopics.length === 0 && (
                <span className="text-xs text-muted-foreground italic">No topics available for this selection</span>
              )}
              {availableTopics.map((topic) => {
                const isSelected = topics.includes(topic);
                return (
                  <Badge
                    key={topic}
                    variant={isSelected ? "default" : "outline"}
                    className={cn(
                      "cursor-pointer transition-colors text-xs font-medium py-0.5",
                      isSelected ? "bg-blue-600 hover:bg-blue-700 text-white border-blue-600" : "hover:bg-accent border-border"
                    )}
                    onClick={() => {
                      setTopics((prev) =>
                        prev.includes(topic)
                          ? prev.filter((t) => t !== topic)
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

          {/* Content Editors */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 flex-1 min-h-[300px]">
            <div className="flex flex-col gap-2 h-full">
              <div>
                <label className="text-sm font-semibold text-foreground">Question Content:</label>
                <p className="text-xs text-muted-foreground">Markdown supported. Inline math: $...$, Block math: $$...$$</p>
              </div>
              <RichTextEditor
                markdown={content}
                onChange={setContent}
                className="flex-1 w-full h-full"
              />
            </div>
            <div className="flex flex-col gap-2 h-full">
              <label className="text-sm font-semibold text-foreground">Mark Scheme Answer (Optional):</label>
              <RichTextEditor
                markdown={answerContent}
                onChange={setAnswerContent}
                placeholder="Type or paste the answer scheme here..."
                className="flex-1 w-full h-full mt-[18px]"
              />
            </div>
          </div>

        </div>

        <div className="flex justify-end gap-2 mt-auto pt-4 border-t border-border shrink-0">
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isSubmitting}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={isSubmitting}>
            {isSubmitting ? "Saving..." : "Save Question"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
