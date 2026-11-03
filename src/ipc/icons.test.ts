import { mockConvertFileSrc } from "@tauri-apps/api/mocks";
import { beforeEach, expect, test } from "vitest";
import { driveIconUrl, IconKey, iconPixels, placeIconUrl, typeIconUrl } from ".";

beforeEach(() => {
  mockConvertFileSrc("windows");
});

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
