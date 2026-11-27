import { describe, expect, it } from "vitest";
import { COMPUTER_NAME_MAX, cleanComputerName, defaultComputerName } from "./computerName";

const CHROME_WIN = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";
const EDGE_WIN = `${CHROME_WIN} Edg/131.0`;
const FIREFOX_LINUX = "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0";
const SAFARI_IPAD = "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";

describe("computer name", () => {
  it("names the platform and the browser", () => {
    expect(defaultComputerName(CHROME_WIN, false)).toBe("Windows · Chrome");
    expect(defaultComputerName(EDGE_WIN, false)).toBe("Windows · Edge");
    expect(defaultComputerName(FIREFOX_LINUX, false)).toBe("Linux · Firefox");
    expect(defaultComputerName(SAFARI_IPAD, false)).toBe("iPad · Safari");
    expect(defaultComputerName(CHROME_WIN, true)).toBe("Windows · desktop app");
    expect(defaultComputerName("", false)).toBe("Web browser");
  });

  it("tidies a typed name", () => {
    expect(cleanComputerName("  Booth \n laptop ")).toBe("Booth laptop");
    expect(cleanComputerName("x".repeat(100))).toHaveLength(COMPUTER_NAME_MAX);
    expect(cleanComputerName("   ")).toBe("");
  });
});
