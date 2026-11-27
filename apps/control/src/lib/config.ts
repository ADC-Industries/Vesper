/**
 * Product naming lives here and only here. "Vesper" is the product family: the server is
 * Vesper and this control app is Vesper Oriel. Window title and first menu use the short
 * `APP_NAME`; the About box, the browser tab and the client name use `APP_FULL_NAME`.
 */
export const FAMILY_NAME = "Vesper";
export const APP_NAME = "Oriel";
export const APP_FULL_NAME = `${FAMILY_NAME} ${APP_NAME}`;
export const SERVER_NAME = FAMILY_NAME;

/** Default API port of the server (see crates/protocol/README.md, Discovery). */
export const DEFAULT_PORT = 5555;

/** localStorage key prefix. */
export const STORAGE_PREFIX = "oriel.";

/** The prefix used before the rename; keys found under it are moved to `STORAGE_PREFIX`.
 * TODO: drop one release after the rename. */
export const LEGACY_STORAGE_PREFIX = "stage.";

/** Client name sent with `hello` and pairing requests. */
export const CLIENT_NAME = APP_FULL_NAME;
