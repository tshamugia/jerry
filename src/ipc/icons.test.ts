import { expect, test } from "vitest";
import { driveIconUrl, entryIconUrl, IconKey, iconPixels, placeIconUrl, typeIconUrl } from ".";

test("icon sizes are device pixels within what the core serves", () => {
  expect(iconPixels(16, 1)).toBe(16);
  expect(iconPixels(16, 1.25)).toBe(20);
  expect(iconPixels(16, 1.5)).toBe(24);
  expect(iconPixels(16, 2)).toBe(32);
  expect(iconPixels(4, 1)).toBe(8);
  expect(iconPixels(256, 2)).toBe(256);
});

test("icon URLs name the icon and its size, and are unique to this page load", () => {
  const folder = typeIconUrl(IconKey.folder, 16);
  expect(folder).toMatch(/^http:\/\/icon\.localhost\/type-1-\d+\?v=[0-9a-f]{8}$/);
  const version = new URL(folder).search;
  expect(typeIconUrl(42, 16)).toBe(`http://icon.localhost/type-42-${iconPixels(16)}${version}`);
  expect(driveIconUrl("C", 16)).toBe(`http://icon.localhost/drive-C-${iconPixels(16)}${version}`);
  expect(placeIconUrl("recyclebin", 16)).toBe(
    `http://icon.localhost/place-recyclebin-${iconPixels(16)}${version}`,
  );
});

test("an entry with its own icon is asked for by entry, any other by its type", () => {
  const own = { listing: 3, id: 1200, key: IconKey.own, modified: 1_767_225_600_000.5 };
  const url = new URL(entryIconUrl(own, 16));
  expect(url.pathname).toBe(`/entry-3-1200-${iconPixels(16)}`);
  expect(url.searchParams.get("m")).toBe("1767225600000");
  expect(entryIconUrl({ ...own, modified: 1 }, 16)).not.toBe(url.href);

  const dwg = { ...own, key: 7 };
  expect(entryIconUrl(dwg, 16)).toBe(typeIconUrl(7, 16));
});
