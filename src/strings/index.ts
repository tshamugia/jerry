/** Every user-visible string lives here. English only, no i18n library (PRD). */
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
    toggleHidden: "Toggle hidden files",
    nextTheme: "Next theme",
    openWindow: "Open another window",
    settingsLoading: "Settings: loading…",
    settings: (showHidden: boolean, theme: string) =>
      `Settings: hidden files ${showHidden ? "shown" : "hidden"}, theme ${theme}.`,
  },
} as const;

export { errorText, fileName, sizeText } from "./errors";
