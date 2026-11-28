import { describe, expect, it } from "vitest";
import type { CcliLicenseStatus } from "$protocol";
import { SHOW_ON, licenceOffer, licenceValue, showOnValue } from "./ccli";

const status = (licenseNumber?: string, importedLicense?: string): CcliLicenseStatus => ({
  settings: { licenseNumber, enabled: true, showOn: "firstSlide" },
  importedLicense,
});

describe("Settings ▸ CCLI", () => {
  it("offers the imported licence number only while none is set", () => {
    expect(licenceOffer(status(undefined, "7654321"))).toBe("7654321");
    expect(licenceOffer(status("  ", " 7654321 "))).toBe("7654321");
    expect(licenceOffer(status("1112223", "7654321"))).toBeNull();
    expect(licenceOffer(status(undefined, undefined))).toBeNull();
    expect(licenceOffer(null)).toBeNull();
  });

  it("keeps a typed number trimmed and an empty one as none", () => {
    expect(licenceValue(" 1112223 ")).toBe("1112223");
    expect(licenceValue("   ")).toBeUndefined();
  });

  it("names the four choices the copyright line has, an unknown one as the first slide", () => {
    expect(SHOW_ON.map((o) => o.value)).toEqual(["firstSlide", "lastSlide", "firstAndLast", "allSlides"]);
    expect(showOnValue("allSlides")).toBe("allSlides");
    expect(showOnValue("unknown")).toBe("firstSlide");
    expect(showOnValue(undefined)).toBe("firstSlide");
  });
});
