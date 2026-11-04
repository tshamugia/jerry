import { expect, test } from "vitest";
import {
  canMoveColumn,
  DEFAULT_COLUMNS,
  MIN_COLUMN_PX,
  moveColumn,
  resizeColumn,
  shownColumns,
  toggleColumn,
} from "./columns";

test("the Details view shows Name, Date modified, Type and Size", () => {
  expect(shownColumns(DEFAULT_COLUMNS)).toEqual(["name", "modified", "type", "size"]);
});

test("optional columns show and hide; Name always shows", () => {
  const withCreated = toggleColumn(DEFAULT_COLUMNS, "created");
  expect(shownColumns(withCreated)).toEqual(["name", "modified", "type", "size", "created"]);

  const withoutType = toggleColumn(withCreated, "type");
  expect(shownColumns(withoutType)).toEqual(["name", "modified", "size", "created"]);

  expect(toggleColumn(DEFAULT_COLUMNS, "name")).toBe(DEFAULT_COLUMNS);
});

test("a column moves past the next shown column, not past hidden ones", () => {
  const moved = moveColumn(DEFAULT_COLUMNS, "size", -1);
  expect(shownColumns(moved)).toEqual(["name", "modified", "size", "type"]);

  // Size is the last shown column: Date created and Extension are hidden.
  expect(canMoveColumn(DEFAULT_COLUMNS, "size", 1)).toBe(false);
  expect(moveColumn(DEFAULT_COLUMNS, "size", 1)).toBe(DEFAULT_COLUMNS);
  expect(canMoveColumn(DEFAULT_COLUMNS, "name", -1)).toBe(false);
  expect(shownColumns(moveColumn(DEFAULT_COLUMNS, "name", 1))[0]).toBe("modified");

  // A hidden column doesn't move.
  expect(moveColumn(DEFAULT_COLUMNS, "created", -1)).toBe(DEFAULT_COLUMNS);
});

test("widths are whole pixels, never below the minimum; Name takes the rest", () => {
  expect(resizeColumn(DEFAULT_COLUMNS, "type", 120.4).widths).toEqual({ type: 120 });
  expect(resizeColumn(DEFAULT_COLUMNS, "size", 3).widths).toEqual({ size: MIN_COLUMN_PX });
  expect(resizeColumn(DEFAULT_COLUMNS, "name", 300)).toBe(DEFAULT_COLUMNS);

  // Resizing keeps the order and the hidden columns as they were.
  const resized = resizeColumn(DEFAULT_COLUMNS, "type", 120);
  expect(resized.order).toBe(DEFAULT_COLUMNS.order);
  expect(resized.hidden).toBe(DEFAULT_COLUMNS.hidden);
});
