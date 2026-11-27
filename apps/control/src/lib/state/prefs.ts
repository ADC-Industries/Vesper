// The one store of remembered servers, pairing tokens and UI preferences (localStorage),
// shared by the state modules.

import { Credentials } from "../server";

export const creds = new Credentials();
