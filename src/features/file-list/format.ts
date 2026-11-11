import { sizeText, strings } from "../../strings";

/** What a zero FILETIME (no time recorded) becomes in ms since the Unix epoch. */
const NO_TIME_MS = -11_644_473_600_000;

const pad = (value: number) => String(value).padStart(2, "0");

/**
 * A date as the "Date format" setting writes it (`DD.MM.YYYY` by default: `DD`, `MM` and `YYYY`
 * are replaced) and the time as `HH:mm`, in local time. Empty when no time is recorded.
 */
export function dateText(ms: number, format: string): string {
  if (ms <= NO_TIME_MS) {
    return "";
  }
  const date = new Date(ms);
  const day = format.replace(/YYYY|MM|DD/g, (token) => {
    switch (token) {
      case "YYYY":
        return String(date.getFullYear());
      case "MM":
        return pad(date.getMonth() + 1);
      default:
        return pad(date.getDate());
    }
  });
  return `${day} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** A file's size as the Size column shows it; folders show none. */
export function sizeCell(bytes: number, folder: boolean): string {
  return folder ? "" : sizeText(bytes);
}

/** What follows the name's last dot, as written; empty for a folder or a name without one. */
export function extensionOf(name: string, folder: boolean): string {
  const dot = name.lastIndexOf(".");
  return folder || dot < 0 ? "" : name.slice(dot + 1);
}

/**
 * The Type column, with the names Explorer gives types with nothing registered, as the core
 * sorts by them (`PlainTypeNames`): "File folder", "File" and "PDF File". Only Latin letters
 * are uppercased: `toUpperCase()` would turn Georgian Mkhedruli into Mtavruli, which Windows
 * doesn't (spike #18).
 */
export function typeName(name: string, folder: boolean): string {
  if (folder) {
    return strings.fileList.folderType;
  }
  const extension = extensionOf(name, false);
  if (extension === "") {
    return strings.fileList.fileType;
  }
  return strings.fileList.typeOf(extension.replace(/[a-z]+/g, (latin) => latin.toUpperCase()));
}

/**
 * Search results' Location column (#66): the folder a result is in, from the folder searched down
 * ("Projects\BMW\Drawings"), which a narrow pane still shows; a folder elsewhere as it is. `root`
 * is the folder searched, as display text.
 */
export function locationText(folder: string, root: string): string {
  const base = root.endsWith("\\") ? root.slice(0, -1) : root;
  if (folder !== base && !folder.startsWith(`${base}\\`)) {
    return folder;
  }
  const name = base.slice(base.lastIndexOf("\\") + 1) || base;
  return name + folder.slice(base.length);
}

/** The Name column: without the extension when "Show file name extensions" is off. */
export function nameText(name: string, folder: boolean, showExtensions: boolean): string {
  const dot = name.lastIndexOf(".");
  return showExtensions || folder || dot <= 0 ? name : name.slice(0, dot);
}
