import { useEffect, useState } from "react";
import { Channel, commands, type FrameInfo, type FrameState, type FrameStateUpdate } from "../ipc";

export type Frame = {
  /** Undefined until the core has answered. */
  info: FrameInfo | undefined;
  state: FrameState;
};

const initialState: FrameState = { hovered: null, pressed: null, maximized: false, active: true };

/**
 * Attaches to the native window frame: learns whether Mica is on (and marks `<html>` with
 * `data-backdrop`) and follows the caption buttons' hover and pressed state, which only the
 * native hit-test sees. `onReady` runs once the backdrop is applied, so the first frame shown
 * already has the right background.
 */
export function useFrame(onReady: () => void): Frame {
  const [info, setInfo] = useState<FrameInfo>();
  const [state, setState] = useState(initialState);

  // biome-ignore lint/correctness/useExhaustiveDependencies: attach once per mount.
  useEffect(() => {
    let attached = true;
    const updates = new Channel<FrameStateUpdate>();
    updates.onmessage = (next) => {
      if (attached) {
        setState({ ...next, hovered: next.hovered ?? null, pressed: next.pressed ?? null });
      }
    };
    commands
      .titleBarAttach(updates)
      .then(
        (frame) => {
          if (!attached) return;
          document.documentElement.dataset.backdrop = frame.mica ? "mica" : "solid";
          setInfo(frame);
        },
        // No native frame (a plain browser): keep the flat background.
        () => undefined,
      )
      .finally(() => {
        if (attached) requestAnimationFrame(onReady);
      });
    return () => {
      attached = false;
    };
  }, []);

  return { info, state };
}
