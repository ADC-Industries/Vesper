import { describe, expect, it } from "vitest";
import type { Element } from "$protocol";
import { placeholderFor, roleLabel, slideRoles, templateSlide } from "./themeTemplates";

let n = 0;
const uuid = () => `00000000-0000-4000-8000-${String(++n).padStart(12, "0")}`;

describe("theme template slides", () => {
  it("builds the starter layouts on the theme's canvas", () => {
    const s = templateSlide("scripture", { width: 1280, height: 720 }, uuid);
    expect(s.label?.text).toBe("Scripture");
    expect(slideRoles(s)).toEqual(["scripture", "reference"]);
    expect(s.elements?.[0]?.bounds).toEqual({ x: 64, y: 36, width: 1152, height: 518.4 });
    expect(new Set(s.elements?.map((e) => e.id)).size).toBe(2);
    expect(templateSlide("blank", undefined, uuid, "Blank 2").elements).toEqual([]);
    expect(templateSlide("blank", undefined, uuid, "Blank 2").label?.text).toBe("Blank 2");
    expect(slideRoles(templateSlide("title", undefined, uuid))).toEqual(["title"]);
  });

  it("labels roles and makes placeholders for the picked one", () => {
    expect(roleLabel("lyrics")).toBe("Lyrics");
    expect(roleLabel("chorusNote")).toBe("chorusNote");
    expect(roleLabel(undefined)).toBe("None");
    const text = { id: "a", kind: { type: "text", text: {} } } as Element;
    const pic = { id: "b" } as Element;
    expect(placeholderFor(text, "title")).toEqual({ slot: "title", accepts: "text" });
    expect(placeholderFor(text, "")).toBeNull();
    expect(placeholderFor(pic, "picture")).toEqual({ slot: "picture", accepts: "media" });
  });
});
