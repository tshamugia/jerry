import {
  type Announcements,
  closestCenter,
  DndContext,
  type DragEndEvent,
  type DragStartEvent,
  KeyboardSensor,
  type Modifier,
  PointerSensor,
  type UniqueIdentifier,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { ReactNode } from "react";
import { strings } from "../../strings";
import type { RenderRow, RowDrag } from "./FavoritesSection";
import { applyDrop, dropOf, type FavoriteRow, useFavorites } from "./favorites";

const t = strings.sidebar;

/** A press becomes a drag past this many pixels; less is a click that opens the favorite. */
const DRAG_DISTANCE = 6;

/** Space picks a line up and puts it down; Enter stays the key that opens. */
const KEYS = { start: ["Space"], end: ["Space"], cancel: ["Escape"] };

/** Lines move only up and down. */
const upAndDown: Modifier = ({ transform }) => ({ ...transform, x: 0 });

/**
 * The Favorites section's lines, draggable to reorder them (dnd-kit; OLE is for files): a
 * favorite within its group or into another, a group with its favorites among the groups.
 * Loaded lazily by the section, which draws the same lines without dragging until then.
 */
export default function SortableFavorites({
  rows,
  render,
  onLift,
}: {
  rows: readonly FavoriteRow[];
  render: RenderRow;
  /** A group is picked up (its favorites hide, to move with it) or put down (undefined). */
  onLift: (group: number | undefined) => void;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: DRAG_DISTANCE } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
      keyboardCodes: KEYS,
    }),
  );
  const ids = rows.map((row) => row.key);
  const name = (id: UniqueIdentifier | undefined) => {
    const row = rows.find((each) => each.key === id);
    return row?.kind === "favorite" ? row.favorite.name : (row?.group.name ?? "");
  };
  const announcements: Announcements = {
    onDragStart: ({ active }) => t.pickedUp(name(active.id)),
    onDragOver: ({ active, over }) =>
      over ? t.movedOver(name(active.id), name(over.id)) : undefined,
    onDragEnd: ({ active }) => t.putDown(name(active.id)),
    onDragCancel: ({ active }) => t.dragCancelled(name(active.id)),
  };
  const start = ({ active }: DragStartEvent) => {
    useFavorites.setState({ dragging: true });
    const row = rows.find((each) => each.key === active.id);
    if (row?.kind === "group") {
      onLift(row.group.id);
    }
  };
  const finish = () => {
    useFavorites.setState({ dragging: false });
    onLift(undefined);
  };
  // `rows` are the lines as drawn while dragging: a lifted group's favorites aren't among them.
  const end = ({ active, over }: DragEndEvent) => {
    if (over && active.id !== over.id) {
      const drop = dropOf(rows, ids.indexOf(String(active.id)), ids.indexOf(String(over.id)));
      if (drop) {
        applyDrop(drop);
      }
    }
    finish();
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
      onDragStart={start}
      onDragEnd={end}
      onDragCancel={finish}
    >
      <SortableContext items={ids} strategy={verticalListSortingStrategy}>
        {rows.map((row, index) => (
          <SortableRow key={row.key} id={row.key}>
            {(drag) => render(row, index, drag)}
          </SortableRow>
        ))}
      </SortableContext>
    </DndContext>
  );
}

function SortableRow({ id, children }: { id: string; children: (drag: RowDrag) => ReactNode }) {
  const { setNodeRef, transform, transition, listeners, attributes, isDragging } = useSortable({
    id,
  });
  return children({
    ref: setNodeRef,
    style: { transform: CSS.Translate.toString(transform), transition },
    attributes: {
      tabIndex: attributes.tabIndex,
      "aria-roledescription": attributes["aria-roledescription"],
      "aria-describedby": attributes["aria-describedby"],
      "aria-pressed": attributes["aria-pressed"],
    },
    // dnd-kit types its listeners loosely; these are a pointer and a key handler.
    onPointerDown: listeners?.onPointerDown as RowDrag["onPointerDown"],
    onKeyDown: listeners?.onKeyDown as RowDrag["onKeyDown"],
    dragging: isDragging,
  });
}
