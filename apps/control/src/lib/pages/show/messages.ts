// Message helpers: tokens in a message's text, keeping token definitions in step with the
// text, and the values sent with `messageShow`.

import type { MessageToken, TokenValue } from "$protocol";

// The server fills in `${name}` and `{name}` for every token a message declares, whatever the
// name's characters (imported names are free text, spaces included). A new token typed here is a
// name that starts and ends with a word character: `{ name }` with padding spaces is plain text.
const TOKEN = /\$?\{([A-Za-z0-9_](?:[A-Za-z0-9_ .-]*[A-Za-z0-9_.-])?)\}/g;

/** Where a token first appears in the text as `${name}` or `{name}` (-1: not used). */
function firstUse(text: string, name: string): number {
  const a = text.indexOf("${" + name + "}");
  const b = text.indexOf("{" + name + "}");
  if (a < 0) return b;
  if (b < 0) return a;
  return Math.min(a, b);
}

/**
 * Token names used in a message text (`${name}` or `{name}`), in first-use order. Declared
 * names count wherever they appear literally, as the server replaces them (D08-1).
 */
export function tokenNames(text: string, declared: string[] = []): string[] {
  const found: string[] = [];
  for (const m of text.matchAll(TOKEN)) {
    const n = m[1]!;
    if (!found.includes(n)) found.push(n);
  }
  for (const n of declared) if (n && !found.includes(n) && firstUse(text, n) >= 0) found.push(n);
  return found
    .map((n) => ({ n, at: firstUse(text, n) }))
    .sort((x, y) => x.at - y.at)
    .map((x) => x.n);
}

/**
 * Token definitions for a text: existing definitions of names still used are kept (timer /
 * clock tokens keep their kind), new names become text tokens with an empty default, names
 * no longer in the text are dropped.
 */
export function syncTokens(text: string, tokens: MessageToken[]): MessageToken[] {
  return tokenNames(
    text,
    tokens.map((t) => t.name),
  ).map((name) => tokens.find((t) => t.name === name) ?? { name, kind: { type: "text" } });
}

/** The value a text token starts with (its default). */
export function defaultValue(t: MessageToken): string {
  return t.kind?.type === "text" ? (t.kind.defaultValue ?? "") : "";
}

/** Values to send: the text tokens whose value differs from their default. */
export function tokenValues(tokens: MessageToken[], values: Record<string, string>): TokenValue[] {
  const out: TokenValue[] = [];
  for (const t of tokens) {
    if (t.kind?.type !== "text" && t.kind !== undefined) continue;
    const v = values[t.name];
    if (v !== undefined && v !== defaultValue(t)) out.push({ name: t.name, value: v });
  }
  return out;
}

/**
 * The text with text tokens filled in (timer / clock tokens shown as their name), the way the
 * server does it: each declared token's `${name}`, then its `{name}`, replaced literally.
 */
export function preview(text: string, tokens: MessageToken[], values: Record<string, string>): string {
  const shown = (t: MessageToken): string => {
    if (t.kind?.type === "timer") return `[${t.name}]`;
    if (t.kind?.type === "clock") return "[clock]";
    return values[t.name] ?? defaultValue(t);
  };
  let out = text;
  for (const t of tokens) if (t.name) out = out.split("${" + t.name + "}").join(shown(t));
  for (const t of tokens) if (t.name) out = out.split("{" + t.name + "}").join(shown(t));
  return out;
}
