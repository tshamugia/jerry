import "react";

declare module "react" {
  interface HTMLAttributes<T> {
    /**
     * Element Timing (WebView2 is Chromium): `performance.getEntriesByType("element")` reports
     * when the element was first painted. The file list's rows carry it, so a start can be timed
     * to its first row (Tech PRD «წარმადობა»).
     */
    elementtiming?: string;
  }
}
