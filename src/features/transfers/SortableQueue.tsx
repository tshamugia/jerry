import {
  type Announcements,
  closestCenter,
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  type Modifier,
  PointerSensor,
  type UniqueIdentifier,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { ReactNode } from "react";
import type { TransferJob } from "../../ipc";
import { strings } from "../../strings";
import { jobTitle } from "./panel";
import type { QueueDrag, RenderJob } from "./TransferQueue";

const t = strings.transfers.panel;

/** A press becomes a drag past this many pixels; less is a click on the line's buttons. */
const DRAG_DISTANCE = 6;

/** Space on the grip picks a line up and puts it down; Escape puts it back. */
const KEYS = { start: ["Space"], end: ["Space"], cancel: ["Escape"] };

/** Lines move only up and down. */
const upAndDown: Modifier = ({ transform }) => ({ ...transform, x: 0 });

/** dnd-kit's id of job `id`. */
function key(id: number): string {
  return `job-${id}`;
}

/**
 * The queue's jobs, draggable to reorder them (dnd-kit; OLE is for files). Loaded lazily by the
 * queue, which draws the same lines without dragging until then. `onOrder` gets the jobs' ids in
 * their new order.
 */
export default function SortableQueue({
  jobs,
  render,
  onOrder,
}: {
  jobs: readonly TransferJob[];
  render: RenderJob;
  onOrder: (ids: number[]) => void;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: DRAG_DISTANCE } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
      keyboardCodes: KEYS,
    }),
  );
  const ids = jobs.map((job) => key(job.id));
  const name = (id: UniqueIdentifier | undefined) => {
    const job = jobs.find((each) => key(each.id) === id);
    return job ? jobTitle(job, false) : "";
  };
  const announcements: Announcements = {
    onDragStart: ({ active }) => t.pickedUp(name(active.id)),
    onDragOver: ({ active, over }) =>
      over ? t.movedOver(name(active.id), name(over.id)) : undefined,
    onDragEnd: ({ active }) => t.putDown(name(active.id)),
    onDragCancel: ({ active }) => t.dragCancelled(name(active.id)),
  };
  const end = ({ active, over }: DragEndEvent) => {
    if (over && active.id !== over.id) {
      const from = ids.indexOf(String(active.id));
      const to = ids.indexOf(String(over.id));
      if (from >= 0 && to >= 0) {
        onOrder(
          arrayMove(
            jobs.map((job) => job.id),
            from,
            to,
          ),
        );
      }
    }
  };
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[upAndDown]}
      accessibility={{
        announcements,
        screenReaderInstructions: { draggable: t.dragInstructions },
      }}
      onDragEnd={end}
    >
      <SortableContext items={ids} strategy={verticalListSortingStrategy}>
        {jobs.map((job) => (
          <SortableJob key={job.id} id={key(job.id)}>
            {(drag) => render(job, drag)}
          </SortableJob>
        ))}
      </SortableContext>
    </DndContext>
  );
}

function SortableJob({ id, children }: { id: string; children: (drag: QueueDrag) => ReactNode }) {
  const {
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    listeners,
    attributes,
    isDragging,
  } = useSortable({ id });
  return children({
    ref: setNodeRef,
    handleRef: setActivatorNodeRef,
    handle: {
      tabIndex: attributes.tabIndex,
      "aria-roledescription": attributes["aria-roledescription"],
      "aria-describedby": attributes["aria-describedby"],
      "aria-pressed": attributes["aria-pressed"],
      // dnd-kit types its listeners loosely; this is a key handler.
      onKeyDown: listeners?.onKeyDown as QueueDrag["handle"]["onKeyDown"],
    },
    style: { transform: CSS.Translate.toString(transform), transition },
    // A press anywhere on the line drags it; a click on its buttons stays a click.
    onPointerDown: listeners?.onPointerDown as QueueDrag["onPointerDown"],
    dragging: isDragging,
  });
}
