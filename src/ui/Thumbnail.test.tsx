import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { Thumbnail } from "./Thumbnail";

const ICON = "http://icon.localhost/type-7-32?v=1";
const THUMB = "http://thumb.localhost/conflict-2-existing-64?v=1&q=1";

function images(container: HTMLElement): HTMLImageElement[] {
  return [...container.querySelectorAll("img")];
}

function shown(src: string | undefined) {
  return render(<Thumbnail src={src} icon={ICON} className="size-16" iconClassName="size-8" />);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

test("the icon shows until the thumbnail is in, then the thumbnail does", () => {
  const { container } = shown(THUMB);
  const [icon, thumb] = images(container);
  expect(icon?.getAttribute("src")).toBe(ICON);
  expect(thumb?.getAttribute("src")).toBe(THUMB);
  expect(thumb).toHaveClass("hidden");

  fireEvent.load(thumb as HTMLImageElement);
  const after = images(container);
  expect(after).toHaveLength(1);
  expect(after[0]?.getAttribute("src")).toBe(THUMB);
  expect(after[0]).not.toHaveClass("hidden");
  expect(after[0]?.getAttribute("srcset")).toMatch(/ \d+(\.\d+)?x$/);
});

test("a file with no thumbnail keeps its icon", () => {
  const { container } = shown(THUMB);
  fireEvent.error(images(container)[1] as HTMLImageElement);
  expect(images(container).map((img) => img.getAttribute("src"))).toEqual([ICON]);
});

test("without a thumbnail URL only the icon shows", () => {
  const { container } = shown(undefined);
  expect(images(container).map((img) => img.getAttribute("src"))).toEqual([ICON]);
});

test("the thumbnail is asked for only once the box is on screen", () => {
  let report: ((entries: { isIntersecting: boolean }[]) => void) | undefined;
  const disconnect = vi.fn();
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(callback: (entries: { isIntersecting: boolean }[]) => void) {
        report = callback;
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  const { container } = shown(THUMB);
  expect(images(container).map((img) => img.getAttribute("src"))).toEqual([ICON]);

  act(() => report?.([{ isIntersecting: false }]));
  expect(images(container)).toHaveLength(1);

  act(() => report?.([{ isIntersecting: true }]));
  expect(images(container).map((img) => img.getAttribute("src"))).toEqual([ICON, THUMB]);
  expect(disconnect).toHaveBeenCalled();
});

test("another file starts over from its icon", () => {
  const { container, rerender } = shown(THUMB);
  fireEvent.load(images(container)[1] as HTMLImageElement);
  const other = THUMB.replace("q=1", "q=2");
  rerender(<Thumbnail src={other} icon={ICON} className="size-16" iconClassName="size-8" />);
  expect(images(container).map((img) => img.getAttribute("src"))).toEqual([ICON, other]);
});
