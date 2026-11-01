import "@testing-library/jest-dom/vitest";
import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(async () => {
  cleanup();
  // Unmounting unlistens Tauri events asynchronously; let that finish before the mocks go.
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});
