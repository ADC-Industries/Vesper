// The name this computer pairs with (the site key reads "Oriel – <name>"), so a site admin can
// tell pairing requests apart: the operator's own, or one made from the platform and browser
// ("Windows · Chrome"; the desktop app: "Windows · desktop app").

/** At most this many characters are sent (the server shortens the key name further). */
export const COMPUTER_NAME_MAX = 40;

/** The platform and browser in words, from a user agent string. */
export function defaultComputerName(userAgent: string, desktopApp: boolean): string {
  const ua = userAgent;
  const os = /Windows/i.test(ua)
    ? "Windows"
    : /iPhone/i.test(ua)
      ? "iPhone"
      : /iPad/i.test(ua)
        ? "iPad"
        : /Android/i.test(ua)
          ? "Android"
          : /CrOS/i.test(ua)
            ? "ChromeOS"
            : /Mac OS X|Macintosh/i.test(ua)
              ? "Mac"
              : /Linux/i.test(ua)
                ? "Linux"
                : "";
  const browser = desktopApp
    ? "desktop app"
    : /Edg\//.test(ua)
      ? "Edge"
      : /OPR\/|Opera/.test(ua)
        ? "Opera"
        : /Firefox\//.test(ua)
          ? "Firefox"
          : /Chrome\/|CriOS\//.test(ua)
            ? "Chrome"
            : /Safari\//.test(ua)
              ? "Safari"
              : "browser";
  return os ? `${os} · ${browser}` : browser === "desktop app" ? "Desktop app" : "Web browser";
}

/** A typed name, tidied: control characters out, spaces collapsed, at most the limit. */
export function cleanComputerName(name: string): string {
  return Array.from(name.replace(/[\u0000-\u001f\u007f]/g, "").replace(/\s+/g, " ").trim())
    .slice(0, COMPUTER_NAME_MAX)
    .join("")
    .trim();
}
