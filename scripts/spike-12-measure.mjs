// Spike #12: drives a running Jerry over the Chrome DevTools Protocol and measures listing,
// sort changes and scrolling for the synthetic folders (1 000, 10 000, 100 000 entries).
//
//   cargo run -p jerry-core --release --example synth          # build the folders once
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333"
//   pnpm tauri dev    (or target\release\jerry.exe)
//   node scripts/spike-12-measure.mjs <label> [port] [out.json]
//
// Timings come from the page itself (window.jerrySpike, src/features/file-list/spike.ts).

import { writeFileSync } from "node:fs";

const label = process.argv[2] ?? "run";
const port = Number(process.argv[3] ?? 9333);
const out = process.argv[4];
const RUNS = Number(process.env.RUNS ?? 5);
const SIZES = (process.env.SIZES ?? "1000,10000,100000").split(",").map(Number);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const print = (line = "") => process.stdout.write(`${line}\n`);

async function pageUrl() {
  for (let attempt = 0; attempt < 240; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // Not up yet.
    }
    await sleep(500);
  }
  throw new Error(`no CDP page on port ${port}`);
}

class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.next = 1;
    this.pending = new Map();
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      const waiter = this.pending.get(message.id);
      if (!waiter) return;
      this.pending.delete(message.id);
      if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
      else waiter.resolve(message.result);
    });
  }
  open() {
    return new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
    });
  }
  send(method, params = {}) {
    const id = this.next++;
    this.socket.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
  close() {
    this.socket.close();
  }
}

async function evaluate(cdp, expression) {
  const timeout = sleep(60_000).then(() => {
    throw new Error(`${expression}: no answer in 60 s (window hidden? rAF stops then)`);
  });
  const result = await Promise.race([
    cdp.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    }),
    timeout,
  ]);
  if (result.exceptionDetails) {
    throw new Error(`${expression}: ${JSON.stringify(result.exceptionDetails)}`);
  }
  return result.result.value;
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
};
const max = (values) => Math.max(...values);
const f = (value, digits = 1) => value.toFixed(digits).padStart(7);

const cdp = new Cdp(await pageUrl());
await cdp.open();
for (let attempt = 0; !(await evaluate(cdp, "!!window.jerrySpike")); attempt++) {
  if (attempt > 120) throw new Error("window.jerrySpike never appeared");
  await sleep(250);
}
const environment = await evaluate(
  cdp,
  "({ ua: navigator.userAgent, dpr: devicePixelRatio, w: innerWidth, h: innerHeight, url: location.href })",
);

const results = { label, runs: RUNS, environment, sizes: {} };

for (const count of SIZES) {
  print(`… ${count}`);
  const r = {};
  r.warmup = await evaluate(cdp, `jerrySpike.open(${count}, "binary")`);
  for (const format of ["binary", "json"]) {
    r[format] = [];
    for (let i = 0; i < RUNS; i++) {
      r[format].push(await evaluate(cdp, `jerrySpike.open(${count}, "${format}")`));
    }
  }
  // Sort changes on a binary listing: every column, both directions, twice.
  await evaluate(cdp, `jerrySpike.open(${count}, "binary")`);
  r.sorts = [];
  for (let round = 0; round < 2; round++) {
    for (const column of ["size", "modified", "type", "name"]) {
      for (const descending of [true, false]) {
        r.sorts.push(await evaluate(cdp, `jerrySpike.sort("${column}", ${descending})`));
      }
    }
  }
  r.scrollLinear = await evaluate(cdp, `jerrySpike.scroll("linear", 4000, 10)`);
  r.scrollJump = await evaluate(cdp, `jerrySpike.scroll("jump", 4000)`);
  // A wheel-like scroll through the compositor (CDP gesture), frames logged in the page.
  const rect = await evaluate(cdp, "jerrySpike.listRect()");
  await evaluate(cdp, "jerrySpike.frameLogStart()");
  await cdp.send("Input.synthesizeScrollGesture", {
    x: Math.round(rect.x + rect.width / 2),
    y: Math.round(rect.y + rect.height / 2),
    yDistance: -Math.min(20000, count * 30 - rect.height - 100),
    speed: 6000,
    gestureSourceType: "mouse",
  });
  r.scrollGesture = await evaluate(cdp, "jerrySpike.frameLogStop()");
  results.sizes[count] = r;
}
cdp.close();

if (out) writeFileSync(out, JSON.stringify(results, null, 2));

print(`\n${label}: median of ${RUNS} runs, ms`);
print(
  "entries  format | first batch  first rows  sorted paint | rust: list   sink    send   index  order | batches   MB   js decode",
);
for (const [count, r] of Object.entries(results.sizes)) {
  for (const format of ["binary", "json"]) {
    const runs = r[format];
    const m = (pick) => median(runs.map(pick));
    print(
      `${count.padStart(7)}  ${format.padEnd(6)} | ${f(m((t) => t.firstBatchMs))}     ${f(m((t) => t.firstRowsMs))}      ${f(m((t) => t.sortedPaintMs))} | ${f(m((t) => t.rust.listMs))} ${f(m((t) => t.rust.sinkMs))} ${f(m((t) => t.rust.sendMs))} ${f(m((t) => t.rust.keysMs + t.rust.nameSortMs + t.rust.ranksMs))} ${f(m((t) => t.rust.orderMs))} | ${String(m((t) => t.rust.batches)).padStart(7)} ${f(m((t) => t.rust.bytes) / 1e6, 2)} ${f(m((t) => t.decodeMs))}`,
    );
  }
}
print("\nsort change (16 per size): total = click to painted");
print("entries | total med  total max | ipc med  rust med  paint med");
for (const [count, r] of Object.entries(results.sizes)) {
  const s = r.sorts;
  print(
    `${count.padStart(7)} | ${f(median(s.map((t) => t.totalMs)))}    ${f(max(s.map((t) => t.totalMs)))} | ${f(median(s.map((t) => t.ipcMs)))}  ${f(median(s.map((t) => t.rustMs)), 2)}   ${f(median(s.map((t) => t.paintMs)))}`,
  );
}
print("\nscroll (4 s each): fps, frame p95/p99/max ms, missed frames (> 1.5 refresh), long tasks");
for (const [count, r] of Object.entries(results.sizes)) {
  for (const [name, s] of [
    ["linear 10 rows/frame", r.scrollLinear],
    ["jump every frame", r.scrollJump],
    ["wheel gesture", r.scrollGesture],
  ]) {
    print(
      `${count.padStart(7)} ${name.padEnd(21)} | ${f(s.fps, 0)} fps (refresh ${s.refreshMs.toFixed(1)} ms) | p95 ${f(s.p95Ms)} p99 ${f(s.p99Ms)} max ${f(s.maxMs)} | missed ${s.missed}/${s.frames} | long ${s.longTasks}`,
    );
  }
}

// Pending evaluate timeouts would keep Node alive for a minute.
process.exit(0);
