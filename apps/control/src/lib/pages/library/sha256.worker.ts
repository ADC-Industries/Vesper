// The upload's SHA-256, off the main thread: receives `{id, file}`, posts `{id, done}` as it
// goes and `{id, sha256}` (or `{id, error}`) at the end.
import { hashBlob } from "./sha256";

self.onmessage = async (e: MessageEvent<{ id: number; file: Blob }>) => {
  const { id, file } = e.data;
  try {
    let last = 0;
    const sha256 = await hashBlob(file, (done) => {
      const now = Date.now();
      if (now - last > 200) {
        last = now;
        self.postMessage({ id, done });
      }
    });
    self.postMessage({ id, sha256 });
  } catch (err) {
    self.postMessage({ id, error: err instanceof Error ? err.message : String(err) });
  }
};
