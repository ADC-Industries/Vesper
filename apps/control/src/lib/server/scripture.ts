// Typed REST client for scripture (crates/protocol/README.md, "Scripture"): translations and
// their books, reference parsing, passages, word search, the slide split of a passage, a
// generated slide's picture and creating the presentation. Plain TypeScript over a RestClient;
// errors stay `ApiRequestError`s.

import type {
  ScriptureBook,
  ScriptureCreate,
  ScriptureCreated,
  ScriptureParse,
  ScripturePassage,
  ScriptureResplitResult,
  ScriptureSearch,
  ScriptureSlideRequest,
  ScriptureSlides,
  ScriptureTranslation,
} from "$protocol";
import type { RestClient } from "./rest";

const enc = encodeURIComponent;

export class ScriptureClient {
  constructor(readonly rest: RestClient) {}

  translations = () => this.rest.get<ScriptureTranslation[]>("/scripture/translations");
  books = (translation: string) => this.rest.get<ScriptureBook[]>(`/scripture/translations/${enc(translation)}/books`);
  parse = (translation: string, q: string) => this.rest.get<ScriptureParse>("/scripture/parse", { translation, q });
  passage = (translation: string, ref: string) => this.rest.get<ScripturePassage>("/scripture/passage", { translation, ref });
  search = (translation: string, q: string, limit = 50) => this.rest.get<ScriptureSearch>("/scripture/search", { translation, q, limit });
  /** The split of a passage into slides (nothing is written). */
  slides = (req: ScriptureSlideRequest) => this.rest.request<ScriptureSlides>("POST", "/scripture/slides", req);
  /** A generated slide's picture for an `<img>` (token in the query). */
  previewUrl = (params: Record<string, string | number>) => this.rest.url("/scripture/preview", params, true);
  /** Creates the presentation (then adds it to a playlist or shows it, as asked). */
  create = (body: ScriptureCreate) => this.rest.request<ScriptureCreated>("POST", "/scripture/presentations", body);
  /** Splits a generated passage again with its theme as it is now; `force` also when it was
   * edited (the edits are replaced; without it such a passage answers 409). */
  resplit = (presentation: string, force = false) =>
    this.rest.request<ScriptureResplitResult>("POST", `/scripture/presentations/${enc(presentation)}/resplit`, { force });
}
