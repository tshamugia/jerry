/**
 * Static tabs and places (spikes #17 and #18); the real models arrive with #23 and the sidebar.
 * Spike #18 fills them with Georgian names: plain, mixed, Mtavruli and too long to fit.
 */
export type MockTab = {
  id: number;
  name: string;
  icon: "folder" | "download";
  pinned?: boolean;
};

export type PaneTabs = { tabs: MockTab[]; activeId: number };

export const initialTabs: { left: PaneTabs; right: PaneTabs } = {
  left: {
    tabs: [
      { id: 1, name: "პროექტები", icon: "folder", pinned: true },
      { id: 2, name: "ქართული სახელები", icon: "folder" },
      { id: 3, name: "Downloads", icon: "download" },
    ],
    activeId: 2,
  },
  right: {
    tabs: [
      { id: 4, name: "ბათუმი 2026 · Tower B", icon: "folder" },
      { id: 5, name: "ფაილები და ნახაზები ძალიან გრძელი სახელით", icon: "folder" },
      { id: 6, name: "ᲜᲐᲮᲐᲖᲔᲑᲘ", icon: "folder" },
    ],
    activeId: 4,
  },
};

export const mockPlaces = [
  "Desktop",
  "Downloads",
  "Documents",
  "ბათუმი 2026",
  "ხელშეკრულებები",
  "OneDrive",
];

/** The right pane's mocked address (display text only). */
export const mockRightPath = "D:\\პროექტები\\ბათუმი 2026\\Tower B\\04_გაცემული";
