/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
  fileList: {
    name: "Name",
    modified: "Date modified",
    type: "Type",
    size: "Size",
    folderType: "File folder",
    fileType: "File",
    extType: (ext: string) => `${ext.toUpperCase()} File`,
    sizeKb: (kb: string) => `${kb} KB`,
    items: (count: number) => `${new Intl.NumberFormat("en-US").format(count)} items`,
    loading: "Loading…",
    error: (code: string) => `Couldn't open the folder (${code}).`,
  },
  spike: {
    open: (count: number) => `Open ${new Intl.NumberFormat("en-US").format(count)}`,
    json: "JSON",
    scroll: "Scroll test",
  },
} as const;
