import { type RefObject, useEffect, useRef, useState } from "react";

/**
 * A file's thumbnail over its type icon (Tech PRD «აიკონები და thumbnail-ები», #67). The icon
 * shows until the thumbnail is in, and stays when the file has none: the core answers 404 for a
 * type with no thumbnail, a folder or a cloud-only file whose thumbnail Windows doesn't have.
 *
 * The thumbnail is asked for only once the box is on screen, and only once; in a virtualized
 * list, rows off screen aren't mounted at all. It fits the box with its aspect ratio kept and is
 * never scaled up: a small picture stays small.
 *
 * `src` is a `thumb://` URL for the box's CSS size (`entryThumbUrl`, `conflictThumbUrl`); none
 * shows the icon only. `className` sizes the box, `iconClassName` the icon.
 */
export function Thumbnail({
  src,
  icon,
  className,
  iconClassName,
}: {
  src: string | undefined;
  icon: string;
  className: string;
  iconClassName: string;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const visible = useOnScreen(box);
  // By URL, so a new file starts over from its icon.
  const [loaded, setLoaded] = useState<string | undefined>(undefined);
  const [failed, setFailed] = useState<string | undefined>(undefined);
  const shown = src !== undefined && loaded === src;
  // The URL asks for device pixels: the descriptor shows them at the box's CSS size.
  const ratio = globalThis.devicePixelRatio || 1;
  return (
    <div ref={box} className={`flex items-center justify-center ${className}`}>
      {!shown && <img src={icon} alt="" className={iconClassName} />}
      {visible && src !== undefined && failed !== src && (
        <img
          src={src}
          srcSet={`${src} ${ratio}x`}
          alt=""
          draggable={false}
          onLoad={() => setLoaded(src)}
          onError={() => setFailed(src)}
          // A ring, not a border: drawn outside, so the picture keeps its exact pixels.
          className={shown ? "max-h-full max-w-full rounded-bar ring-1 ring-stroke" : "hidden"}
        />
      )}
    </div>
  );
}

/** Whether `element` has been on screen. Without `IntersectionObserver` (tests), it has. */
function useOnScreen(element: RefObject<HTMLElement | null>): boolean {
  const [seen, setSeen] = useState(typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const target = element.current;
    if (seen || !target) {
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        setSeen(true);
        observer.disconnect();
      }
    });
    observer.observe(target);
    return () => observer.disconnect();
  }, [element, seen]);
  return seen;
}
