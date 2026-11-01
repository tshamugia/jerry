import { strings } from "../../strings";

const sizeFormat = new Intl.NumberFormat("en-US");

/** `DD.MM.YYYY HH:mm` in local time (the default date format). */
export function formatDate(unixMs: number): string {
  const d = new Date(unixMs);
  return `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function pad(value: number): string {
  return value < 10 ? `0${value}` : String(value);
}

/** Explorer-style size: whole KB, rounded up. */
export function formatSize(bytes: number): string {
  return strings.fileList.sizeKb(sizeFormat.format(Math.ceil(bytes / 1024)));
}

/** The Type column: "File folder", "PDF File", "File". */
export function formatType(name: string, isDir: boolean): string {
  if (isDir) {
    return strings.fileList.folderType;
  }
  const dot = name.lastIndexOf(".");
  return dot < 0 ? strings.fileList.fileType : strings.fileList.extType(name.slice(dot + 1));
}
