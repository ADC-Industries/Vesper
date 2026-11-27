import { ApiRequestError, CommandError } from "../server";

/** A failure in plain words, for toasts and dialogs. */
export function errorText(e: unknown): string {
  if (e instanceof ApiRequestError || e instanceof CommandError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
