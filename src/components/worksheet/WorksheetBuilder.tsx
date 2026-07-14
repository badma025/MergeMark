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
import { FileText, Clock, Hash, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
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
}

export function WorksheetBuilder({ selectedQuestions, onRemove, onReorder }: WorksheetBuilderProps) {
  const totalMarks = selectedQuestions.reduce((acc, q) => acc + q.marks, 0);
  const estMinutes = totalMarks; // 1 min per mark heuristic
  const [isCompiling, setIsCompiling] = useState(false);

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
    try {
      const filePath = await invoke<string>("compile_worksheet", { questionIds: ids });
      toast.success("Worksheet compiled!", {
        description: filePath,
        duration: 6000,
      });
    } catch (err) {
      toast.error("Compilation failed", {
        description: String(err),
        duration: 8000,
      });
    } finally {
      setIsCompiling(false);
    }
  }

  return (
    <aside
      className={cn(
        "flex w-80 flex-shrink-0 flex-col border-l border-border bg-background",
        "h-screen" // full viewport height
      )}
      aria-label="Worksheet Builder"
    >
      {/* ── Header ── */}
      <div className="flex flex-col gap-1 border-b border-border px-4 pt-4 pb-3">
        <div className="flex items-center gap-2">
          <FileText className="size-4 text-primary flex-shrink-0" />
          <h2 className="text-sm font-semibold tracking-tight text-foreground">
            Current Worksheet
          </h2>
        </div>
        <p className="text-[0.7rem] text-muted-foreground pl-6">
          {selectedQuestions.length} question{selectedQuestions.length !== 1 ? "s" : ""}
        </p>
      </div>

      {/* ── Stats row ── */}
      <div className="flex gap-2 px-4 py-3 border-b border-border">
        <StatChip icon={Hash}  label="Total Marks" value={`${totalMarks}`} />
        <StatChip icon={Clock} label="Est. Time"   value={`${estMinutes}m`} />
      </div>

      {/* ── Scrollable sortable list ── */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
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

      {/* ── Pinned compile button ── */}
      <div className="border-t border-border px-4 py-3 bg-background">
        <Button
          id="compile-pdf-btn"
          className={cn(
            "w-full gap-2 font-semibold",
            "bg-primary text-primary-foreground hover:bg-primary/90",
            "shadow-lg shadow-primary/20 transition-all duration-200",
            "hover:shadow-primary/30 active:shadow-none"
          )}
          onClick={handleCompile}
          disabled={selectedQuestions.length === 0 || isCompiling}
          aria-label="Compile worksheet to PDF"
        >
          {isCompiling ? (
            <>
              <Loader2 className="size-4 animate-spin" />
              Compiling…
            </>
          ) : (
            <>
              <FileText className="size-4" />
              Compile PDF
            </>
          )}
        </Button>
      </div>
    </aside>
  );
}
