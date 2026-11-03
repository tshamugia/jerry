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
  },
} as const;

export { errorText, fileName, sizeText } from "./errors";
