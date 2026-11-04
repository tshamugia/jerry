import "@testing-library/jest-dom/vitest";
import { clearMocks, mockConvertFileSrc } from "@tauri-apps/api/mocks";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach } from "vitest";

// Protocol URLs as WebView2 builds them (`http://icon.localhost/…`): icons render everywhere.
beforeEach(() => {
  mockConvertFileSrc("windows");
});

afterEach(() => {
  cleanup();
  clearMocks();
});
