import type { JerryError } from "../ipc";

/**
 * The English text for an error from the core: what went wrong, naming the file when the error
 * has one. The only place error codes become text (CLAUDE.md, architecture rule 5).
 */
export function errorText(error: JerryError): string {
  const name = error.path === null ? undefined : quoted(fileName(error.path));
  switch (error.code) {
    case "AccessDenied":
      return name
        ? `You don't have permission to access ${name}.`
        : "You don't have permission to do this.";
    case "NotFound":
      return name
        ? `${name} couldn't be found. It may have been moved or deleted.`
        : "The file or folder couldn't be found. It may have been moved or deleted.";
    case "InUse": {
      const app = error.params.app ?? "another program";
      return `${name ?? "The file"} is open in ${app}. Close it there and try again.`;
    }
    case "NoSpace": {
      const { needed, available } = error.params;
      return needed === null || available === null
        ? "There isn't enough space on the drive."
        : `There isn't enough space on the drive: ${sizeText(needed)} needed, ${sizeText(available)} free.`;
    }
    case "InvalidName":
      return name ? `${name} isn't a valid name here.` : "This name isn't valid here.";
    case "DeviceGone":
      return name
        ? `The drive or network location of ${name} is no longer available.`
        : "The drive or network location is no longer available.";
    case "NoRecycleBin":
      return name
        ? `${name} can't go to the Recycle Bin. Deleting it would be permanent.`
        : "This location has no Recycle Bin. Deleting here is permanent.";
    case "Cancelled":
      return "Cancelled.";
    case "Os":
      return name
        ? `Windows couldn't complete this for ${name} (${osCodeText(error.params.os)}).`
        : `Windows couldn't complete this (${osCodeText(error.params.os)}).`;
    case "Internal":
      return "Something went wrong in Jerry. Details are in the log.";
  }
}

/** The last part of a Windows path: `C:\Work\plan.dwg` → `plan.dwg`, `C:\` → `C:`. */
export function fileName(path: string): string {
  const trimmed = path.replace(/\\+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("\\") + 1) || path;
}

function quoted(name: string): string {
  return `“${name}”`;
}

/** Win32 codes in decimal (`error 5`), `HRESULT`s in hex (`error 0x80270064`). */
function osCodeText(code: number): string {
  return code >= 0x8000_0000 ? `error 0x${code.toString(16).toUpperCase()}` : `error ${code}`;
}

const SIZE_UNITS = ["bytes", "KB", "MB", "GB", "TB", "PB"] as const;
const sizeNumber = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

/** A size the way Explorer shows free space: 1024-based, one decimal (`12.5 GB`). */
export function sizeText(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < SIZE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${sizeNumber.format(value)} ${SIZE_UNITS[unit]}`;
}
