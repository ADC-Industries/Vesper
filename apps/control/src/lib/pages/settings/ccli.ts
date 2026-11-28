// Settings ▸ CCLI (`GET` / `PUT /settings/ccli`): the choices of where the copyright line shows
// (the server's `CcliDisplay`, counted in a song's play order) and the licence number offered
// from imported SongSelect files.

import type { CcliDisplay, CcliLicenseStatus } from "$protocol";

export const SHOW_ON: { value: CcliDisplay; label: string }[] = [
  { value: "firstSlide", label: "First slide" },
  { value: "lastSlide", label: "Last slide" },
  { value: "firstAndLast", label: "First and last slide" },
  { value: "allSlides", label: "Every slide" },
];

/** The slides a value names (an unknown one is the server's first slide). */
export function showOnValue(v: CcliDisplay | undefined): CcliDisplay {
  return SHOW_ON.some((o) => o.value === v) ? v! : "firstSlide";
}

/** The licence number to offer ("Use … from your imported songs"): the one imported SongSelect
 * files carried, while no number is set. */
export function licenceOffer(s: CcliLicenseStatus | null): string | null {
  if (!s || s.settings.licenseNumber?.trim()) return null;
  return s.importedLicense?.trim() || null;
}

/** A typed licence number as the server keeps it: trimmed, nothing for empty. */
export function licenceValue(text: string): string | undefined {
  const t = text.trim();
  return t ? t : undefined;
}
