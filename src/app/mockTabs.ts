/** Static tabs and places for spike #17; the real models arrive with #23 and the sidebar. */
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
      { id: 1, name: "PROJECTS", icon: "folder", pinned: true },
      { id: 2, name: "Drawings", icon: "folder" },
      { id: 3, name: "Downloads", icon: "download" },
    ],
    activeId: 2,
  },
  right: {
    tabs: [
      { id: 4, name: "04_Issued", icon: "folder" },
      { id: 5, name: "ფაილები და ნახაზები", icon: "folder" },
    ],
    activeId: 4,
  },
};

export const mockPlaces = ["Desktop", "Downloads", "Documents", "Pictures", "OneDrive"];
