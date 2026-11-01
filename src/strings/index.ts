/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
} as const;
