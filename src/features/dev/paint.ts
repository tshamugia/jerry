/**
 * Resolves with `performance.now()` just after the next frame is painted: a `requestAnimationFrame`
 * callback runs right before the paint, and a message posted from it runs right after. Commit
 * first (`flushSync`), then call this to time what the user sees.
 */
export function afterNextPaint(): Promise<number> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => {
        channel.port1.close();
        resolve(performance.now());
      };
      channel.port2.postMessage(null);
    });
  });
}
