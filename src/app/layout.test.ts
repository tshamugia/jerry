import { describe, expect, test } from "vitest";
import {
  dragPreview,
  dragSidebar,
  dragSplit,
  frameColumns,
  layoutZones,
  MIN_PANE,
  SIDEBAR_LIMITS,
} from "./layout";

const design = { sidebar: 232, preview: 288, split: 0.5 };

describe("layoutZones", () => {
  test("the design's window shows every zone at the design's sizes", () => {
    const zones = layoutZones(1440, design);
    expect(zones).toEqual({ sidebar: 232, preview: 288, split: 0.5 });
    expect(frameColumns(zones)).toBe("232px minmax(0, 0.5fr) 1px minmax(0, 0.5fr) 288px");
  });

  test("a narrow window hides the preview first, then the sidebar", () => {
    // Two panes need 2 × 240 + 1 = 481 px.
    expect(layoutZones(232 + 288 + 481, design)).toMatchObject({ sidebar: 232, preview: 288 });
    expect(layoutZones(232 + 288 + 480, design)).toMatchObject({ sidebar: 232, preview: 0 });
    expect(layoutZones(232 + 481, design)).toMatchObject({ sidebar: 232, preview: 0 });
    expect(layoutZones(232 + 480, design)).toMatchObject({ sidebar: 0, preview: 0 });
    expect(frameColumns(layoutZones(600, design))).toBe("minmax(0, 0.5fr) 1px minmax(0, 0.5fr)");
  });

  test("at the minimum window width (800) the sidebar still shows", () => {
    expect(layoutZones(800, design)).toMatchObject({ sidebar: 232, preview: 0 });
  });

  test("sizes outside the limits are clamped, and both panes keep their minimum", () => {
    const zones = layoutZones(1440, { sidebar: 5000, preview: 10, split: 0.99 });
    expect(zones.sidebar).toBe(SIDEBAR_LIMITS.max);
    expect(zones.preview).toBe(200);
    const panes = 1440 - zones.sidebar - zones.preview - 1;
    expect(panes * (1 - zones.split)).toBeCloseTo(MIN_PANE);
    expect(layoutZones(1440, { ...design, split: Number.NaN }).split).toBe(0.5);
  });
});

describe("drags", () => {
  const zones = layoutZones(1440, design);

  test("the sidebar edge follows the pointer within its limits", () => {
    expect(dragSidebar(300, 1440, zones)).toBe(300);
    expect(dragSidebar(20, 1440, zones)).toBe(SIDEBAR_LIMITS.min);
    expect(dragSidebar(2000, 1440, zones)).toBe(SIDEBAR_LIMITS.max);
    // A narrower window leaves the panes their minimum.
    expect(dragSidebar(400, 1000, zones)).toBe(1000 - 288 - 481);
  });

  test("the preview edge follows the pointer from the right", () => {
    expect(dragPreview(1440 - 320, 1440, zones)).toBe(320);
    expect(dragPreview(1430, 1440, zones)).toBe(200);
  });

  test("the divider sets the left pane's share", () => {
    const panes = 1440 - 232 - 288 - 1;
    expect(dragSplit(232 + panes * 0.4, 1440, zones)).toBe(0.4);
    expect(dragSplit(0, 1440, zones)).toBeCloseTo(MIN_PANE / panes, 3);
  });
});
