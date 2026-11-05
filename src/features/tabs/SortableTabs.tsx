import {
  closestCenter,
  DndContext,
  type DragEndEvent,
  type Modifier,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { horizontalListSortingStrategy, SortableContext, useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { ReactNode } from "react";
import { moveTab, type PaneTab } from "../../stores/tabs";
import type { PaneSide } from "../../stores/window";
import type { RenderTab, TabDrag } from "./TabStrip";

/** A press becomes a drag past this many pixels; less is a click that shows the tab. */
const DRAG_DISTANCE = 6;

/** Tabs move only sideways, along the strip. */
const sideways: Modifier = ({ transform }) => ({ ...transform, y: 0 });

/**
 * A pane's tabs, draggable to reorder them within the window (dnd-kit; OLE is for files). A tab
 * stays in its group: a pinned one among the pinned, another after them (`moveTab`). Loaded
 * lazily by the tab strip, which draws the same tabs without dragging until then.
 */
export default function SortableTabs({
  pane,
  tabs,
  render,
}: {
  pane: PaneSide;
  tabs: readonly PaneTab[];
  render: RenderTab;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: DRAG_DISTANCE } }),
  );
  const ids = tabs.map((tab) => tab.id);
  const end = ({ active, over }: DragEndEvent) => {
    if (over && active.id !== over.id) {
      moveTab(pane, Number(active.id), ids.indexOf(Number(over.id)));
    }
  };
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[sideways]}
      onDragEnd={end}
    >
      <SortableContext items={ids} strategy={horizontalListSortingStrategy}>
        {tabs.map((tab, index) => (
          <SortableTab key={tab.id} id={tab.id}>
            {(drag) => render(tab, index, drag)}
          </SortableTab>
        ))}
      </SortableContext>
    </DndContext>
  );
}

function SortableTab({ id, children }: { id: number; children: (drag: TabDrag) => ReactNode }) {
  const { setNodeRef, transform, transition, listeners, isDragging } = useSortable({ id });
  return children({
    ref: setNodeRef,
    style: { transform: CSS.Translate.toString(transform), transition },
    // dnd-kit types its listeners loosely; this one is a pointer handler.
    onPointerDown: listeners?.onPointerDown as TabDrag["onPointerDown"],
    dragging: isDragging,
  });
}
