/** Every user-visible string lives here. English only, no i18n library (PRD). */

const integer = new Intl.NumberFormat("en-US");
const decimal = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
  /** The dev panel (dev builds only). */
  dev: {
    title: "Dev checks",
    sampleError: "Sample error",
    panicInJob: "Panic in a job",
    blockMainThread: "Freeze main thread",
    blockPage: "Freeze page",
    stream: "Binary stream",
    noError: "The command didn't fail.",
    frozeMainThread: (ms: number) => `Froze the main thread for ${ms} ms. The log has a warning.`,
    frozePage: (ms: number) => `Froze the page for ${ms} ms. The log has a warning.`,
    streamed: (messages: number, values: string) =>
      `Received ${messages} binary messages: ${values}.`,
    streamIncomplete: (messages: number) => `Only ${messages} of 3 messages arrived.`,
    listingTitle: "Listing",
    listingPath: "Folder path",
    listFolder: "List",
    listSynth: (count: number) => `List ${integer.format(count)}`,
    buildingSynth: (count: number) => `Building a folder with ${integer.format(count)} items…`,
    listing: "Listing…",
    items: (count: number) => `${integer.format(count)} items`,
    listed: (timings: {
      count: number;
      batches: number;
      firstBatchMs: number;
      firstPaintMs: number;
      doneMs: number;
      donePaintMs: number;
      rustMs: number;
    }) =>
      `${integer.format(timings.count)} items in ${timings.batches} ${timings.batches === 1 ? "batch" : "batches"}. ` +
      `First rows painted ${decimal.format(timings.firstPaintMs)} ms, all ${decimal.format(timings.donePaintMs)} ms ` +
      `(first batch ${decimal.format(timings.firstBatchMs)} ms, done ${decimal.format(timings.doneMs)} ms, ` +
      `Rust ${decimal.format(timings.rustMs)} ms).`,
    sidebarIcons: "Sidebar icons",
    places: {
      desktop: "Desktop",
      documents: "Documents",
      downloads: "Downloads",
      pictures: "Pictures",
      onedrive: "OneDrive",
      recyclebin: "Recycle Bin",
      share: "Network share",
    },
    drive: (letter: string) => `${letter}:`,
    scrollTest: "Scroll test",
    scrolling: "Scrolling…",
    scrolled: (result: {
      rows: number;
      ms: number;
      frames: number;
      medianMs: number;
      p95Ms: number;
      maxMs: number;
      janky: number;
      longTasks: number;
      fromMemory: number;
      fromShell: number;
      rendered: number;
      failed: number;
    }) =>
      `Scrolled ${integer.format(result.rows)} rows in ${decimal.format(result.ms / 1000)} s: ` +
      `${integer.format(result.frames)} frames, median ${decimal.format(result.medianMs)} ms, ` +
      `95th percentile ${decimal.format(result.p95Ms)} ms, longest ${decimal.format(result.maxMs)} ms, ` +
      `${result.janky} janky, ${result.longTasks} long tasks. Icons: ${integer.format(result.fromMemory)} from memory, ` +
      `${integer.format(result.fromShell)} from the Shell, ${integer.format(result.rendered)} rendered, ${result.failed} failed.`,
  },
} as const;

export { errorText, fileName, sizeText } from "./errors";
