// Spike #11: turns startup_probe runs (JSON lines) into stage tables.
//
//   node scripts/spike-11-summary.mjs <runs.jsonl> [more.jsonl ...]
//
// Every time is ms from the moment the probe called CreateProcess ("launch"). Rust stamps,
// UI stamps (timeOrigin + performance.now()) and the probe all read the system clock.

import { readFileSync } from "node:fs";

const files = process.argv.slice(2);
if (files.length === 0) {
  process.stderr.write("usage: node scripts/spike-11-summary.mjs <runs.jsonl> ...\n");
  process.exit(1);
}

const runs = files
  .flatMap((file) => readFileSync(file, "utf8").split("\n"))
  .filter((line) => line.trim())
  .map((line) => JSON.parse(line))
  .filter((run) => run.recorded)
  // Split each label by the discrete GPU's power state at launch, when the probe recorded it.
  .map((run) => (run.dgpuState ? { ...run, label: `${run.label} · dGPU ${run.dgpuState}` } : run));

const print = (line = "") => process.stdout.write(`${line}\n`);
const median = (values) => {
  const sorted = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (sorted.length === 0) return Number.NaN;
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};
const fmt = (v) => (Number.isFinite(v) ? v.toFixed(0) : "-");
const luma = (hex) => {
  const n = Number.parseInt(hex.slice(1), 16);
  return ((n >> 16) * 299 + ((n >> 8) & 255) * 587 + (n & 255) * 114) / 1000;
};

/** Stage times of one run, ms from launch. */
function stages(run) {
  const t0 = run.launched;
  const report = run.report;
  const s = {};
  const rel = (at) => (Number.isFinite(at) ? at - t0 : Number.NaN);
  s["probe: CreateProcess returned"] = rel(run.spawned);
  if (report) {
    s["kernel: process created"] = rel(report.processCreated);
    for (const { name, at } of report.rust) s[`rust: ${name}`] = rel(at);
    const startup = report.phases.find((p) => p.phase === "startup") ?? report.phases[0];
    // UI stamps accumulate, so the last phase has them all.
    for (const { name, at } of report.phases.at(-1)?.js ?? []) s[`ui: ${name}`] = rel(at);
    if (startup) {
      const procs = startup.processes;
      const browser = procs.find((p) => p.parent === report.pid && /webview/i.test(p.name));
      if (browser) {
        s["webview2: browser process created"] = rel(browser.created);
        const children = procs
          .filter((p) => p.parent === browser.pid)
          .map((p) => p.created)
          .sort((a, b) => a - b);
        children.forEach((created, i) => {
          s[`webview2: child ${i + 1} created`] = rel(created);
        });
      }
    }
  }
  s["probe: window exists"] = rel(run.windowFound);
  s["probe: window visible"] = rel(run.visible);
  s["probe: first rows on screen"] = rel(run.text);
  return s;
}

const labels = [...new Set(runs.map((run) => run.label))];
for (const label of labels) {
  const group = runs.filter((run) => run.label === label);
  const all = group.map(stages);
  const names = [...new Set(all.flatMap((s) => Object.keys(s)))];
  const rows = names
    .map((name) => {
      const values = all.map((s) => s[name]);
      return { name, med: median(values), min: Math.min(...values), max: Math.max(...values) };
    })
    .filter((row) => Number.isFinite(row.med))
    .sort((a, b) => a.med - b.med);
  const cpu = group.map((run) => run.cpuBefore);
  print(
    `\n## ${label}: ${group.length} runs, profile ${group[0].profile}, CPU before ${fmt(Math.min(...cpu))}–${fmt(Math.max(...cpu))} %${group[0].purgeMs != null ? ", standby list purged" : ""}`,
  );
  print(`${"stage".padEnd(44)} ${"median".padStart(7)} ${"min".padStart(6)} ${"max".padStart(6)}`);
  for (const row of rows) {
    print(
      `${row.name.padEnd(44)} ${fmt(row.med).padStart(7)} ${fmt(row.min).padStart(6)} ${fmt(row.max).padStart(6)}`,
    );
  }

  // Per run: when the window was shown, the first frame after the rows, the first row painted.
  print(
    `per run (shown / rows rAF / first row painted / on screen): ${all
      .map((st) =>
        [
          st["rust: shown"],
          st["ui: rows:raf"],
          st["ui: element:first-row"],
          st["probe: first rows on screen"],
        ]
          .map(fmt)
          .join("/"),
      )
      .join("  ")}`,
  );

  // What the screen showed at the title bar patch once the window was visible.
  const flashes = group.map((run) => {
    const after = run.patch.filter(([at]) => at >= (run.visible ?? 0));
    const light = after.filter(([, color]) => luma(color) > 200);
    return {
      colors: after.map(([at, c]) => `${c}@${fmt(at - run.launched)}`),
      light: light.length,
    };
  });
  print(`screen at the title bar after show (first run): ${flashes[0]?.colors.join(" → ") || "-"}`);
  print(`runs with a light (white) frame after show: ${flashes.filter((f) => f.light > 0).length}`);
  print(
    `window became foreground in ${group.filter((run) => run.foreground != null).length} of ${group.length} runs${group[0].highQos ? `; high QoS forced on ${median(group.map((run) => run.boosted))} processes` : ""}`,
  );
  const samples = group.map((run) => run.samples);
  const gaps = group.map((run) => run.sampleGapMaxMs);
  print(
    `probe: ${fmt(median(samples))} samples per run (median), largest gap ${fmt(Math.max(...gaps))} ms`,
  );

  const values = group.flatMap((run) =>
    (run.report?.phases ?? []).flatMap((p) => p.values.map((v) => [v.name, v.at])),
  );
  const byName = new Map();
  for (const [name, v] of values) byName.set(name, [...(byName.get(name) ?? []), v]);
  if (byName.size > 0) {
    print(
      `values (median): ${[...byName].map(([name, vs]) => `${name} ${median(vs).toFixed(1)}`).join(", ")}`,
    );
  }

  // Memory per phase: Jerry and WebView2, private working set (Task Manager) and commit.
  const phases = new Map();
  for (const run of group) {
    for (const phase of run.report?.phases ?? []) {
      const sum = (pick) => phase.processes.reduce((total, p) => total + pick(p), 0) / 2 ** 20;
      const jerry = phase.processes.find((p) => p.pid === run.report.pid);
      phases.set(phase.phase, [
        ...(phases.get(phase.phase) ?? []),
        {
          privateWs: sum((p) => p.privateWorkingSet),
          commit: sum((p) => p.privateBytes),
          jerryWs: (jerry?.privateWorkingSet ?? 0) / 2 ** 20,
          count: phase.processes.length,
        },
      ]);
    }
  }
  for (const [phase, list] of phases) {
    print(
      `memory ${phase}: ${fmt(median(list.map((m) => m.privateWs)))} MB private working set (jerry.exe ${fmt(median(list.map((m) => m.jerryWs)))} MB), ${fmt(median(list.map((m) => m.commit)))} MB commit, ${median(list.map((m) => m.count))} processes`,
    );
  }
}
