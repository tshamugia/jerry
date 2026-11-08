import { expect, test } from "vitest";
import { shortChanges } from "./notes";

/** What `pnpm exec git-cliff --latest --strip header` writes (cliff.toml). */
const notes = `## [0.2.0](https://github.com/tshamugia/jerry/releases/tag/v0.2.0) (2026-11-20)

### Features

- Copy and move ([d6354a2](https://github.com/tshamugia/jerry/commit/d6354a23f415f85ccaf72cefaaac41b774e791c7))
- **transfers:** Conflict dialog ([abcdef1](https://github.com/tshamugia/jerry/commit/abcdef1))

### Fixes

- Reading \`.dwg\` names, see [the guide](https://example.com) ([1234567](https://github.com/tshamugia/jerry/commit/1234567))
`;

test("release notes become plain list items", () => {
  expect(shortChanges(notes)).toEqual({
    changes: ["Copy and move", "transfers: Conflict dialog", "Reading .dwg names, see the guide"],
    more: 0,
  });
});

test("only the first few are listed and the rest counted", () => {
  const many = Array.from({ length: 9 }, (_, index) => `- Change ${index + 1}`).join("\n");
  const { changes, more } = shortChanges(many);
  expect(changes).toEqual(["Change 1", "Change 2", "Change 3", "Change 4"]);
  expect(more).toBe(5);
});

test("Windows line endings, repeats and other list marks", () => {
  expect(shortChanges("* One\r\n* One\r\n- Two\r\n")).toEqual({
    changes: ["One", "Two"],
    more: 0,
  });
});

test("notes without a list have no changes to show", () => {
  expect(shortChanges("")).toEqual({ changes: [], more: 0 });
  expect(shortChanges("## 0.2.0\n\nNo changes a user would notice.\n")).toEqual({
    changes: [],
    more: 0,
  });
});

test("markup in the notes stays text", () => {
  expect(shortChanges("- <img src=x onerror=alert(1)>").changes).toEqual([
    "<img src=x onerror=alert(1)>",
  ]);
});
