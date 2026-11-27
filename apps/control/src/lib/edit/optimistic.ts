// A value the inspector just sent, shown until its own save settled (sent, refetched, or failed
// and reloaded). Controls that send a whole value (a slide's builds, a transition, a text's
// scrolling) build the next change on top of it, so two changes in one gesture (a number
// committed on pointerdown, then a click) don't undo each other (D10-2). An earlier save's
// refetch doesn't drop it, so a third quick change still builds on the second (E10-5).

export interface Sent<T> {
  /** What the value belongs to (a slide, an element, a presentation, a look: its ID). Another
   *  target never shows it, also when both have no value of their own (E10-6). */
  key: string;
  value: T;
}

/** The value to show and build on: the one just sent for `key`, else the stored one. */
export function shown<T>(sent: Sent<T> | null, key: string, stored: T): T {
  return sent && sent.key === key ? sent.value : stored;
}
