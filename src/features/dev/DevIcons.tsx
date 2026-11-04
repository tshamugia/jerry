import { useState } from "react";
import { driveIconUrl, type Place, placeIconUrl } from "../../ipc";
import { strings } from "../../strings";

const ICON_PX = 16;
const PLACES: Place[] = [
  "desktop",
  "documents",
  "downloads",
  "pictures",
  "onedrive",
  "recyclebin",
  "share",
];
const LETTERS = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"];

/**
 * Dev builds only: the sidebar's icons (standard places, every drive) next to their names, to
 * compare with Explorer's. Not the sidebar (#34). Letters with no drive answer 404 and are hidden.
 */
export function DevIcons() {
  const [gone, setGone] = useState<ReadonlySet<string>>(new Set());
  const hide = (letter: string) => () => setGone((hidden) => new Set(hidden).add(letter));

  return (
    <ul
      aria-label={strings.dev.sidebarIcons}
      className="flex flex-wrap items-center gap-x-4 gap-y-1"
    >
      {PLACES.map((place) => (
        <li key={place} className="flex items-center gap-1">
          <img src={placeIconUrl(place, ICON_PX)} alt="" draggable={false} className="size-4" />
          {strings.dev.places[place]}
        </li>
      ))}
      {LETTERS.filter((letter) => !gone.has(letter)).map((letter) => (
        <li key={letter} className="flex items-center gap-1">
          <img
            src={driveIconUrl(letter, ICON_PX)}
            alt=""
            draggable={false}
            onError={hide(letter)}
            className="size-4"
          />
          {strings.dev.drive(letter)}
        </li>
      ))}
    </ul>
  );
}
