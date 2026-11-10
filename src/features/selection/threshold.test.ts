import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, test, vi } from "vitest";

afterEach(() => {
  clearMocks();
  vi.resetModules();
});

/** A fresh copy of the module, whose core answers `drag_threshold` with `answer`. */
async function withCore(answer: () => unknown) {
  const asked: string[] = [];
  mockIPC((cmd) => {
    asked.push(cmd);
    return answer();
  });
  const { pastDragThreshold } = await import("./threshold");
  return { pastDragThreshold, asked };
}

test("Windows' 4 px until the core answers, then the user's threshold, asked once", async () => {
  const { pastDragThreshold, asked } = await withCore(() => ({ x: 10, y: 8 }));

  expect(pastDragThreshold(3, -3)).toBe(false);
  expect(pastDragThreshold(-4, 0)).toBe(true);
  await vi.waitFor(() => expect(pastDragThreshold(-4, 0)).toBe(false));

  expect(pastDragThreshold(9, 7)).toBe(false);
  expect(pastDragThreshold(10, 0)).toBe(true);
  expect(pastDragThreshold(0, -8)).toBe(true);
  expect(asked).toEqual(["drag_threshold"]);
});

test("without an answer, Windows' default stays", async () => {
  const { pastDragThreshold, asked } = await withCore(() => {
    throw new Error("no core");
  });
  expect(pastDragThreshold(0, 4)).toBe(true);
  await vi.waitFor(() => expect(asked).toEqual(["drag_threshold"]));
  expect(pastDragThreshold(3, 3)).toBe(false);
  expect(pastDragThreshold(0, 4)).toBe(true);
});
