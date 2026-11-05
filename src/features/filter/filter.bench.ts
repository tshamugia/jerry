import { expect, test } from "vitest";
import { matchingRows, visibleRows } from "../../stores/entries";
import { SYNTH_PATTERNS, synthEntries, synthNames } from "./synth.testing";
import { wildcardMatcher } from "./wildcard";

// The quick filter over 100 000 names, one pattern at a time, as each key typed runs it: the
// PRD's budget is one frame (16 ms). `pnpm bench` runs it; the regular tests keep a looser check.
const entries = synthEntries(synthNames(100_000));
const all = visibleRows(entries, undefined, false);

for (const pattern of SYNTH_PATTERNS) {
  test(`quick filter, 100 000 names: ${pattern}`, async ({ bench }) => {
    const result = await bench(pattern, () => {
      matchingRows(entries, all, wildcardMatcher(pattern));
    }).run();
    expect(result.latency.p99).toBeLessThan(16);
  });
}
