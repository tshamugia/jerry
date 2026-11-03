// Compares the criterion results of a pull request's base and head (#44). The benchmark job runs
// both on one runner in interleaved rounds (base-1, head-1, base-2, head-2, ...) and saves each
// round as a criterion baseline. A side's value is its fastest round, so one noisy round can't
// fake a slowdown. A round's value is criterion's estimate, the middle number its log prints.
//
// A benchmark is slower when its head is more than the threshold slower than its base AND every
// head round is slower than every base round. A bigger change inside the rounds' spread is noise:
// shown in the table, without a warning (short IO benchmarks on a busy machine spread 60%).
//
// Writes a Markdown table to the job summary and a warning annotation for every slower benchmark.
// A slowdown never fails the job; missing head results do.
//
//   node .github/scripts/bench-compare.mjs <criterion dir> [--threshold 0.2] [--base <label>]
//       [--head <label>]

import { appendFileSync, existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const ROUND = /^(base|head)-\d+$/;
const UNITS = [
  [1e9, "s"],
  [1e6, "ms"],
  [1e3, "µs"],
];

const args = parseArgs(process.argv.slice(2));
const results = collect(args.dir);
const ids = [...results.keys()].sort((a, b) => a.localeCompare(b, "en", { numeric: true }));
const hasBase = ids.some((id) => results.get(id).base.length > 0);
const hasHead = ids.some((id) => results.get(id).head.length > 0);

if (!hasHead) {
  write(`::error title=No benchmark results::No head-<round> results under ${args.dir}.\n`);
  process.exit(1);
}

const rounds = Math.max(...ids.map((id) => results.get(id).head.length));
const roundsText = `${rounds} round${rounds === 1 ? "" : "s"}`;
const percent = Math.round(args.threshold * 100);
const lines = [];
const slower = [];

if (hasBase) {
  lines.push(
    `### Benchmarks: ${args.head} vs ${args.base}`,
    "",
    `Fastest of ${roundsText} per side, base and head in turns on one runner. A benchmark more`,
    `than ${percent}% slower than the base, with every head round slower than every base round, gets`,
    "a warning. Noise: how far each side's rounds spread.",
    "",
    "| Benchmark | Base | Head | Change | Noise (base / head) |",
    "| --- | --: | --: | --: | --: |",
  );
  for (const id of ids) {
    const { base, head } = results.get(id);
    if (base.length === 0) {
      lines.push(`| \`${id}\` | | ${duration(best(head))} | new | |`);
      continue;
    }
    if (head.length === 0) {
      lines.push(`| \`${id}\` | ${duration(best(base))} | | removed | |`);
      continue;
    }
    const change = best(head) / best(base) - 1;
    const over = change > args.threshold;
    const clear = best(head) > Math.max(...base);
    if (over && clear) slower.push({ id, base: best(base), head: best(head), change });
    const shown = !over
      ? signed(change)
      : clear
        ? `**${signed(change)}** ⚠️`
        : `${signed(change)} (noise)`;
    lines.push(
      `| \`${id}\` | ${duration(best(base))} | ${duration(best(head))} | ${shown} | ${spread(base)} / ${spread(head)} |`,
    );
  }
  lines.push(
    "",
    slower.length > 0
      ? `**${slower.length} of ${ids.length} benchmarks are more than ${percent}% slower than the base.**`
      : `No benchmark is more than ${percent}% slower than the base.`,
  );
} else {
  // The job's own notice says why: no benchmarks on the base yet, or they don't build.
  lines.push(
    `### Benchmarks: ${args.head}`,
    "",
    `No base results to compare with (${args.base}). Fastest of ${roundsText}.`,
    "",
    "| Benchmark | Head | Noise |",
    "| --- | --: | --: |",
  );
  for (const id of ids) {
    const { head } = results.get(id);
    lines.push(`| \`${id}\` | ${duration(best(head))} | ${spread(head)} |`);
  }
}

for (const { id, base, head, change } of slower) {
  const message = `${id}: ${duration(base)} → ${duration(head)} (${signed(change)}), over ${percent}%.`;
  write(`::warning title=Benchmark slower than the base::${escapeCommand(message)}\n`);
}

const summary = `${lines.join("\n")}\n`;
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
}
write(summary);

function parseArgs(argv) {
  const parsed = { dir: undefined, threshold: 0.2, base: "base", head: "head" };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--threshold") parsed.threshold = Number(argv[++i]);
    else if (arg === "--base") parsed.base = argv[++i];
    else if (arg === "--head") parsed.head = argv[++i];
    else parsed.dir = arg;
  }
  if (!parsed.dir || !(parsed.threshold > 0)) {
    write("usage: bench-compare.mjs <criterion dir> [--threshold 0.2] [--base x] [--head y]\n");
    process.exit(2);
  }
  return parsed;
}

/** Every `<side>-<round>` baseline under `dir`: benchmark id → each side's round values (ns). */
function collect(dir) {
  const found = new Map();
  const walk = (folder) => {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      const path = join(folder, entry.name);
      const estimatesPath = join(path, "estimates.json");
      if (ROUND.test(entry.name) && existsSync(estimatesPath)) {
        const id = readJson(join(path, "benchmark.json")).full_id;
        // Criterion's own choice: the slope of linear sampling, else the mean.
        const estimates = readJson(estimatesPath);
        const typical = (estimates.slope ?? estimates.mean).point_estimate;
        const sides = found.get(id) ?? { base: [], head: [] };
        sides[entry.name.split("-")[0]].push(typical);
        found.set(id, sides);
      } else {
        walk(path);
      }
    }
  };
  if (existsSync(dir)) walk(dir);
  return found;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function best(values) {
  return Math.min(...values);
}

/** How far the rounds spread: (slowest − fastest) / fastest. */
function spread(values) {
  return values.length < 2 ? "" : `${Math.round((Math.max(...values) / best(values) - 1) * 100)}%`;
}

function signed(change) {
  const value = Math.round(change * 100);
  return `${value > 0 ? "+" : ""}${value}%`;
}

/** Nanoseconds as s, ms, µs or ns with three significant digits. */
function duration(ns) {
  const [scale, unit] = UNITS.find(([scale]) => ns >= scale) ?? [1, "ns"];
  return `${(ns / scale).toPrecision(3)} ${unit}`;
}

/** Escapes a workflow command's message (`%`, CR, LF). */
function escapeCommand(message) {
  return message.replaceAll("%", "%25").replaceAll("\r", "%0D").replaceAll("\n", "%0A");
}

function write(text) {
  process.stdout.write(text);
}
