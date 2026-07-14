import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, X } from "lucide-react";
import { cn } from "@/lib/utils";

export interface WorksheetItemData {
  id: string;
  subject: string;
  subtopic: string;
  marks: number;
}

interface WorksheetItemProps {
  item: WorksheetItemData;
  onRemove: (id: string) => void;
}

export function WorksheetItem({ item, onRemove }: WorksheetItemProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: item.id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <li
      ref={setNodeRef}
      style={style}
      className={cn(
        "group flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-2.5",
        "select-none transition-shadow duration-150",
        isDragging
          ? "z-50 shadow-lg shadow-black/30 opacity-90 border-primary/50"
          : "hover:border-border/80"
      )}
      aria-label={`${item.subject} – ${item.subtopic}, ${item.marks} marks`}
    >
      {/* Drag handle */}
      <button
        {...attributes}
        {...listeners}
        className={cn(
          "flex-shrink-0 cursor-grab active:cursor-grabbing rounded p-0.5",
          "text-muted-foreground/40 hover:text-muted-foreground",
          "transition-colors duration-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        )}
        aria-label="Drag to reorder"
        tabIndex={0}
      >
        <GripVertical className="size-4" />
      </button>

      {/* Topic info */}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-xs font-semibold text-foreground leading-tight">
          {item.subject}
        </span>
        <span className="truncate text-[0.68rem] text-muted-foreground leading-tight">
          {item.subtopic}
        </span>
      </div>

      {/* Marks pill */}
      <span
        className={cn(
          "flex-shrink-0 rounded-full px-2 py-0.5 text-[0.68rem] font-bold",
          "bg-primary/15 text-primary border border-primary/20"
        )}
        aria-label={`${item.marks} marks`}
      >
        {item.marks}m
      </span>

      {/* Remove button */}
      <button
        onClick={() => onRemove(item.id)}
        className={cn(
          "flex-shrink-0 rounded p-0.5",
          "text-muted-foreground/40 hover:text-destructive",
          "transition-colors duration-100",
          "opacity-0 group-hover:opacity-100",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100"
        )}
        aria-label={`Remove ${item.subject} from worksheet`}
      >
        <X className="size-3.5" />
      </button>
    </li>
  );
}
