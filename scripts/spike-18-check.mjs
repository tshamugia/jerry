// Spike #18: drives a running Jerry over the Chrome DevTools Protocol and checks Georgian
// rendering (fonts per zone, clipped ink, row heights, screenshots per font stack), shortcuts
// by physical key on the en-US and ka-GE layouts (synthetic key events), the quick filter and
// the natural order of the Georgian folder.
//
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9338"
//   $env:WEBVIEW2_USER_DATA_FOLDER="$env:TEMP\jerry-spike-18\webview"
//   $env:JERRY_NO_MICA="1"      (screenshots can't capture Mica: it comes out white)
//   target\debug\jerry.exe      (pnpm tauri build --debug --no-bundle)
//   node scripts/spike-18-check.mjs [port] [outDir]
//
// The page side is window.jerryGeorgian (src/features/georgian/spike.ts). Exits 1 if a check fails.

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const port = Number(process.argv[2] ?? 9338);
const outDir = process.argv[3] ?? "spike-18-out";
mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const print = (line = "") => process.stdout.write(`${line}\n`);
const failures = [];
const check = (ok, what) => {
  print(`${ok ? "PASS" : "FAIL"}  ${what}`);
  if (!ok) failures.push(what);
};

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
  const result = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(`${expression}: ${JSON.stringify(result.exceptionDetails)}`);
  }
  return result.result.value;
}

// --- Fonts ---------------------------------------------------------------------------------

const ZONES = {
  list: '[data-pane="left"] [data-zone="list"] [data-name]',
  tab: "[data-tab-label]",
  breadcrumb: '[data-pane="left"] [data-segment]',
  titlebar: "[data-app-name]",
  sidebar: '[data-zone="sidebar"] [data-name]',
  windowTitle: "[data-window-title]",
};

const GEORGIAN = /[Ⴀ-ჿᲐ-Ჿⴀ-⴯]/u;

async function fontsByZone(cdp) {
  const { root } = await cdp.send("DOM.getDocument", { depth: -1 });
  const report = {};
  for (const [zone, selector] of Object.entries(ZONES)) {
    const { nodeIds } = await cdp.send("DOM.querySelectorAll", { nodeId: root.nodeId, selector });
    const texts = await evaluate(
      cdp,
      `Array.from(document.querySelectorAll(${JSON.stringify(selector)}), (e) => e.textContent)`,
    );
    report[zone] = [];
    for (const [i, nodeId] of nodeIds.entries()) {
      const { fonts } = await cdp.send("CSS.getPlatformFontsForNode", { nodeId });
      report[zone].push({
        text: texts[i],
        fonts: fonts.map(
          (f) => `${f.familyName}${f.isCustomFont ? " (custom)" : ""}:${f.glyphCount}`,
        ),
        families: fonts.map((f) => f.familyName),
      });
    }
  }
  return report;
}

// --- Keys ----------------------------------------------------------------------------------

// Values from the real "Georgian (QWERTY)" layout (ToUnicodeEx): Ctrl keeps the base character.
const LAYOUTS = {
  "en-US": { KeyC: "c", KeyV: "v", KeyF: "f", KeyN: "N" },
  "ka-GE": { KeyC: "ც", KeyV: "ვ", KeyF: "ფ", KeyN: "N" },
};
const SHORTCUTS = [
  { id: "copy", code: "KeyC", vk: 67, shift: false },
  { id: "paste", code: "KeyV", vk: 86, shift: false },
  { id: "filter", code: "KeyF", vk: 70, shift: false },
  { id: "newFolder", code: "KeyN", vk: 78, shift: true },
];
const CTRL = 2;
const SHIFT = 8;

async function pressShortcut(cdp, key, { code, vk, shift }) {
  const modifiers = CTRL | (shift ? SHIFT : 0);
  const down = (params) => cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...params });
  const up = (params) => cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...params });
  const ctrl = { key: "Control", code: "ControlLeft", windowsVirtualKeyCode: 17 };
  const shiftKey = { key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16 };
  const main = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk };
  await down({ ...ctrl, modifiers: CTRL });
  if (shift) await down({ ...shiftKey, modifiers });
  await down({ ...main, modifiers });
  await up({ ...main, modifiers });
  if (shift) await up({ ...shiftKey, modifiers: CTRL });
  await up({ ...ctrl, modifiers: 0 });
}

async function plainKey(cdp, key, code, vk) {
  for (const type of ["rawKeyDown", "keyUp"]) {
    await cdp.send("Input.dispatchKeyEvent", {
      type,
      key,
      code,
      windowsVirtualKeyCode: vk,
      nativeVirtualKeyCode: vk,
    });
  }
}

const blur = (cdp) => evaluate(cdp, "document.activeElement?.blur(); true");
const lastFired = (cdp) => evaluate(cdp, "window.jerryGeorgian.fired().at(-1) ?? null");

// --- Run -----------------------------------------------------------------------------------

const cdp = new Cdp(await pageUrl());
await cdp.open();
await cdp.send("DOM.enable");
await cdp.send("CSS.enable");
for (let i = 0; i < 120; i++) {
  if (await evaluate(cdp, "!!window.jerryGeorgian && window.jerryGeorgian.names().length > 0")) {
    break;
  }
  await sleep(250);
}
const report = {};

print("== Natural sort");
const names = await evaluate(cdp, "window.jerryGeorgian.names()");
report.order = names;
print(`${names.length} entries; first 12: ${names.slice(0, 12).join(" | ")}`);
const numbered = [
  "ნახაზი 1.dwg",
  "ნახაზი 2.dwg",
  "ნახაზი 007.dwg",
  "ნახაზი 9.dwg",
  "ნახაზი 10.dwg",
  "ნახაზი 11.dwg",
  "ნახაზი 100.dwg",
];
const positions = numbered.map((n) => names.indexOf(n));
check(
  positions.every((p, i) => p >= 0 && (i === 0 || p > positions[i - 1])),
  `numbered Georgian names in natural order (${positions.join(", ")})`,
);
const folders = ["ბათუმი 2026", "ნახაზები", "პროექტი 2", "პროექტი 10", "ფოტოები და ვიდეო"];
check(
  names.slice(0, folders.length).join("|") === folders.join("|"),
  `folders first, in Georgian order: ${names.slice(0, folders.length).join(" | ")}`,
);
check(
  names.some((n) => n.includes("�")),
  "the unpaired-surrogate name is listed (shown with U+FFFD)",
);

print("\n== Fonts per zone (Jerry stack)");
await evaluate(cdp, "window.jerryGeorgian.setFont('jerry')");
const fonts = await fontsByZone(cdp);
report.fonts = fonts;
for (const [zone, items] of Object.entries(fonts)) {
  const families = new Map();
  for (const item of items) {
    for (const f of item.fonts) {
      const [family, count] = f.split(":");
      families.set(family, (families.get(family) ?? 0) + Number(count));
    }
  }
  print(`${zone.padEnd(11)} ${[...families].map(([f, n]) => `${f} ${n}`).join(", ")}`);
  for (const item of items) {
    if (!GEORGIAN.test(item.text)) continue;
    const fallback = item.families.filter((f) => !f.startsWith("Segoe UI"));
    if (fallback.length > 0) print(`            ${item.text}  →  ${item.fonts.join(", ")}`);
  }
  const georgianOk = items
    .filter((item) => GEORGIAN.test(item.text))
    .every(
      (item) => item.families.includes("Segoe UI") || item.families.includes("Segoe UI Semibold"),
    );
  check(georgianOk, `${zone}: Georgian text drawn with Segoe UI (the stack's own fallback)`);
  const sylfaen = items.some((item) => item.families.includes("Sylfaen"));
  check(!sylfaen, `${zone}: no Sylfaen fallback`);
}

print("\n== Ink and boxes");
const ink = await evaluate(cdp, "window.jerryGeorgian.ink()");
report.ink = ink;
const clipped = ink.filter((i) => i.clipped);
for (const c of clipped) {
  print(
    `  clipped ${c.zone}: ${c.text} (ink ${c.top.toFixed(1)}..${c.bottom.toFixed(1)} in ${c.box})`,
  );
}
check(clipped.length === 0, `no text ink leaves its box (${ink.length} texts)`);
const georgianInk = ink.filter((i) => GEORGIAN.test(i.text));
const span = (key) => georgianInk.map((i) => i[key]);
print(
  `  Georgian ink: top ${Math.min(...span("top")).toFixed(1)} px, bottom ${Math.max(...span("bottom")).toFixed(1)} px, box ${Math.min(...span("box"))}–${Math.max(...span("box"))} px`,
);
const truncated = ink.filter((i) => i.truncated).map((i) => `${i.zone}: ${i.text.slice(0, 40)}…`);
print(`  truncated with an ellipsis: ${truncated.length}`);
for (const t of truncated) print(`    ${t}`);
const rows = await evaluate(cdp, "window.jerryGeorgian.rowHeights()");
check(rows.length === 1 && rows[0] === 30, `every list row is 30 px (${rows.join(", ")})`);

print("\n== Casing");
const casing = await evaluate(cdp, "window.jerryGeorgian.casing()");
report.casing = casing;
print(`  "ფაილი".toUpperCase() = ${casing.upper}; toLocaleUpperCase("ka") = ${casing.upperKa}`);
print(`  "ᲜᲐᲮᲐᲖᲘ".toLowerCase() = ${casing.lowerMtavruli}; Type column = ${casing.typeColumn}`);
check(casing.typeColumn === "ფაილი File", "Type column keeps Georgian extensions in Mkhedruli");

print("\n== Window title");
const title = await evaluate(cdp, "window.jerryGeorgian.windowTitle()");
report.windowTitle = title;
check(title === "ქართული სახელები — Jerry", `native title round trip: ${title}`);

print("\n== Shortcuts (synthetic CDP key events)");
report.keys = [];
for (const [layout, keys] of Object.entries(LAYOUTS)) {
  for (const shortcut of SHORTCUTS) {
    await blur(cdp);
    await evaluate(cdp, "window.jerryGeorgian.clear()");
    const key = keys[shortcut.code];
    await pressShortcut(cdp, key, shortcut);
    await sleep(30);
    const fired = await lastFired(cdp);
    const focusFilter =
      shortcut.id === "filter"
        ? await evaluate(cdp, "document.activeElement?.dataset.filter !== undefined")
        : true;
    report.keys.push({ layout, ...shortcut, key, fired });
    check(
      fired?.id === shortcut.id &&
        fired?.key === key &&
        fired?.code === shortcut.code &&
        focusFilter,
      `${layout} ${shortcut.id}: key "${key}" code ${shortcut.code} → ${fired ? `${fired.title}` : "nothing"}${shortcut.id === "filter" ? ` (filter focused: ${focusFilter})` : ""}`,
    );
  }
}

print("\n== In the quick filter (ka-GE)");
await blur(cdp);
await evaluate(cdp, "window.jerryGeorgian.clear()");
await pressShortcut(cdp, "ფ", SHORTCUTS[2]);
await cdp.send("Input.insertText", { text: "ნახაზ" });
await sleep(50);
const filteredNames = await evaluate(cdp, "window.jerryGeorgian.names()");
report.filtered = filteredNames;
print(`  "ნახაზ" shows ${filteredNames.length}: ${filteredNames.join(" | ")}`);
check(
  filteredNames.includes("ᲜᲐᲮᲐᲖᲘ ᲡᲐᲑᲝᲚᲝᲝ.dwg") && filteredNames.includes("ნახაზი 10.dwg"),
  "filter matches Mkhedruli and Mtavruli names",
);
const shotFilter = await cdp.send("Page.captureScreenshot", { format: "png" });
writeFileSync(join(outDir, "filter-ka.png"), Buffer.from(shotFilter.data, "base64"));
await evaluate(cdp, "window.jerryGeorgian.clear()");
await pressShortcut(cdp, "ც", SHORTCUTS[0]);
await pressShortcut(cdp, "ვ", SHORTCUTS[1]);
check((await lastFired(cdp)) === null, "Ctrl+C / Ctrl+V in the filter stay the field's own");
await plainKey(cdp, "Escape", "Escape", 27);
const cleared = await evaluate(cdp, "window.jerryGeorgian.names().length");
check(cleared === names.length, `Esc clears the filter (${cleared} shown)`);
await blur(cdp);
await evaluate(cdp, "window.jerryGeorgian.clear()");
await plainKey(cdp, "ც", "KeyC", 67);
check((await lastFired(cdp)) === null, 'plain "ც" (no Ctrl) runs nothing');

print("\n== Screenshots per font stack");
const layout = await cdp.send("Page.getLayoutMetrics");
const { clientWidth, clientHeight } = layout.cssLayoutViewport;
const regions = await evaluate(
  cdp,
  `(() => {
    const r = (s) => { const b = document.querySelector(s)?.getBoundingClientRect(); return b && { x: b.x, y: b.y, width: b.width, height: b.height }; };
    return { titlebar: r('[data-zone="titlebar"]'), breadcrumb: r('[data-pane="left"] [data-zone="breadcrumb"]'), list: r('[data-pane="left"] [data-zone="list"]') };
  })()`,
);
for (const font of ["jerry", "segoe", "sylfaen"]) {
  await evaluate(cdp, `window.jerryGeorgian.setFont(${JSON.stringify(font)})`);
  await sleep(100);
  const full = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, `window-${font}.png`), Buffer.from(full.data, "base64"));
  for (const [name, r] of Object.entries(regions)) {
    if (!r) continue;
    const height = name === "list" ? Math.min(r.height, 520) : r.height;
    const shot = await cdp.send("Page.captureScreenshot", {
      format: "png",
      clip: { x: r.x, y: r.y, width: Math.min(r.width, clientWidth), height, scale: 2 },
    });
    writeFileSync(join(outDir, `${name}-${font}.png`), Buffer.from(shot.data, "base64"));
  }
}
await evaluate(cdp, "window.jerryGeorgian.setFont('jerry')");
print(`  ${clientWidth}×${clientHeight} viewport; PNGs in ${outDir}`);

writeFileSync(join(outDir, "report.json"), JSON.stringify(report, null, 2));
cdp.close();
print(`\n${failures.length === 0 ? "ALL PASS" : `${failures.length} FAILED`}`);
process.exit(failures.length === 0 ? 0 : 1);
