// One IntersectionObserver shared by every thumbnail of a grid.

export interface LazyObserver {
  /** Calls `onVisible` once when `node` comes near the viewport; returns a cleanup. */
  observe(node: Element, onVisible: () => void): () => void;
  disconnect(): void;
}

export function createLazyObserver(root: Element | null, margin = "600px 0px"): LazyObserver {
  const callbacks = new Map<Element, () => void>();
  if (typeof IntersectionObserver === "undefined") {
    // No observer (very old engines, tests): load everything.
    return {
      observe(_node, onVisible) {
        onVisible();
        return () => {};
      },
      disconnect() {},
    };
  }
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        const cb = callbacks.get(e.target);
        if (cb) {
          callbacks.delete(e.target);
          io.unobserve(e.target);
          cb();
        }
      }
    },
    { root, rootMargin: margin },
  );
  return {
    observe(node, onVisible) {
      callbacks.set(node, onVisible);
      io.observe(node);
      return () => {
        callbacks.delete(node);
        io.unobserve(node);
      };
    },
    disconnect() {
      io.disconnect();
      callbacks.clear();
    },
  };
}
