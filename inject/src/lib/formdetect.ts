// Form detection (spec 11 §3): field classes from autocomplete, type and
// name patterns, and the kind of each form. Pure DOM reading, so tests run
// it on fixture pages in jsdom; `forms.ts` adds visibility and reports.

import { NEW_PASSWORD, OTP_WORDS, PATTERNS, SIGN_UP } from "./patterns";

export type FieldClass =
  | "username"
  | "email"
  | "password"
  | "newPassword"
  | "otp"
  | "ccName"
  | "ccNumber"
  | "ccExp"
  | "ccExpMonth"
  | "ccExpYear"
  | "ccCsc"
  | "givenName"
  | "familyName"
  | "name"
  | "organization"
  | "addressLine1"
  | "addressLine2"
  | "addressLine3"
  | "city"
  | "region"
  | "postalCode"
  | "country"
  | "tel"
  | "bday"
  | "unknown";

export type FormKind =
  | "login"
  | "usernameOnly"
  | "signup"
  | "changePassword"
  | "otp"
  | "card"
  | "address"
  | "identity";

export type Field = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;

export interface Detected {
  /** The `<form>`, or the document for fields outside any form. */
  root: HTMLFormElement | Document;
  kind: FormKind;
  fields: { el: Field; cls: FieldClass }[];
}

const AUTOCOMPLETE: Record<string, FieldClass> = {
  username: "username",
  email: "email",
  "current-password": "password",
  "new-password": "newPassword",
  "one-time-code": "otp",
  "cc-name": "ccName",
  "cc-given-name": "ccName",
  "cc-family-name": "ccName",
  "cc-number": "ccNumber",
  "cc-exp": "ccExp",
  "cc-exp-month": "ccExpMonth",
  "cc-exp-year": "ccExpYear",
  "cc-csc": "ccCsc",
  "given-name": "givenName",
  "family-name": "familyName",
  name: "name",
  organization: "organization",
  "street-address": "addressLine1",
  "address-line1": "addressLine1",
  "address-line2": "addressLine2",
  "address-line3": "addressLine3",
  "address-level2": "city",
  "address-level1": "region",
  "postal-code": "postalCode",
  country: "country",
  "country-name": "country",
  tel: "tel",
  "tel-national": "tel",
  bday: "bday",
};

const SKIPPED_TYPES = new Set([
  "hidden",
  "submit",
  "button",
  "reset",
  "image",
  "checkbox",
  "radio",
  "file",
  "range",
  "color",
  "search",
]);

/** Whether a control can hold a value Mado might fill. */
export function fillable(el: Element): el is Field {
  if (el instanceof HTMLInputElement) return !SKIPPED_TYPES.has(el.type.toLowerCase());
  return el instanceof HTMLSelectElement;
}

const clean = (s: string | null | undefined) => (s ?? "").replace(/\s+/g, " ").trim();

/** The text a person reads for the field: its labels, then aria. */
export function labelText(el: Field): string {
  const parts: string[] = [];
  for (const l of el.labels ?? []) parts.push(clean(l.textContent));
  const by = el.getAttribute("aria-labelledby");
  if (by) {
    for (const id of by.split(/\s+/))
      parts.push(clean(el.ownerDocument.getElementById(id)?.textContent));
  }
  return parts.filter(Boolean).join(" ");
}

/** What the name patterns read. Labels first: they are written for people. */
function haystacks(el: Field): string[] {
  return [
    labelText(el),
    clean(el.getAttribute("aria-label")),
    clean(el.getAttribute("placeholder")),
    clean(el.getAttribute("name")),
    clean(el.id),
  ].filter(Boolean);
}

/** The first matching rule of spec 11 §3. */
export function classifyField(el: Field): FieldClass {
  const tokens = clean(el.getAttribute("autocomplete")).toLowerCase().split(" ");
  for (let i = tokens.length - 1; i >= 0; i--) {
    const c = AUTOCOMPLETE[tokens[i] ?? ""];
    if (c) return c;
  }
  const type = el instanceof HTMLInputElement ? el.type.toLowerCase() : "";
  const texts = haystacks(el);
  // "Email or phone": an identifier, whatever its type says.
  const [, either] = PATTERNS[0] ?? [];
  if (type !== "password" && either && texts.some((t) => either.test(t))) return "username";
  if (type === "password") {
    return texts.some((t) => NEW_PASSWORD.test(t)) ? "newPassword" : "password";
  }
  if (type === "email") return "email";
  for (const text of texts) {
    for (const [cls, re] of PATTERNS) if (re.test(text)) return cls;
  }
  if (type === "tel") return "tel";
  // A short numeric field named a code.
  if (
    el instanceof HTMLInputElement &&
    (el.inputMode === "numeric" || type === "number" || el.pattern.includes("\\d")) &&
    el.maxLength >= 4 &&
    el.maxLength <= 8 &&
    texts.some((t) => OTP_WORDS.test(t))
  ) {
    return "otp";
  }
  return "unknown";
}

const ADDRESS = new Set<FieldClass>([
  "addressLine1",
  "addressLine2",
  "addressLine3",
  "city",
  "region",
  "postalCode",
  "country",
]);

/** The form's own words: headings, legends and buttons inside it. */
function formText(root: HTMLFormElement | Document): string {
  const scope = root instanceof Document ? root.body : root;
  if (!scope) return "";
  const parts: string[] = [];
  for (const el of scope.querySelectorAll(
    'h1, h2, h3, legend, button, input[type="submit"], [role="button"]',
  )) {
    parts.push(el instanceof HTMLInputElement ? el.value : clean(el.textContent));
  }
  if (root instanceof HTMLFormElement) {
    parts.push(
      clean(root.getAttribute("action")),
      clean(root.id),
      clean(root.getAttribute("name")),
    );
  }
  return parts.join(" ");
}

/** Words or autocomplete that mark the password a person has now. */
const CURRENT =
  /current|\bold\b|existing|actuel|ancien|actual|aktuell|alte?s?\b|attuale|huidig|atual|現在|当前|текущий/i;

function isCurrent(el: Field): boolean {
  return (
    /current-password/i.test(el.getAttribute("autocomplete") ?? "") ||
    haystacks(el).some((t) => CURRENT.test(t))
  );
}

/** The kind of form these fields make, or null (spec 11 §3, first rule). */
export function classifyForm(
  fields: { el: Field; cls: FieldClass }[],
  root: HTMLFormElement | Document,
): FormKind | null {
  const count = (c: FieldClass) => fields.filter((f) => f.cls === c).length;
  const passwords = count("password") + count("newPassword");
  const ids = count("username") + count("email");
  const current = fields.some((f) => f.cls === "password" && isCurrent(f.el));
  if (passwords >= 3 || (current && count("newPassword") >= 1)) {
    return "changePassword";
  }
  if (count("newPassword") >= 1 || passwords === 2) return "signup";
  if (passwords === 1) {
    const named = count("givenName") + count("familyName") + count("name");
    return named > 0 && SIGN_UP.test(formText(root)) ? "signup" : "login";
  }
  if (count("otp") >= 1) return "otp";
  if (count("ccNumber") >= 1) return "card";
  if (fields.filter((f) => ADDRESS.has(f.cls)).length >= 3) return "address";
  const named = count("givenName") + count("familyName") + count("name");
  if (named > 0 && (count("email") > 0 || count("tel") > 0)) return "identity";
  if (ids === 1 && fields.length <= 2) return "usernameOnly";
  return null;
}

/** After classifying alone: a "security code" on a card form is the CVC. */
function fixUp(fields: { el: Field; cls: FieldClass }[]): void {
  if (!fields.some((f) => f.cls === "ccNumber")) return;
  for (const f of fields) {
    if (f.cls === "otp" && /security|cvc|cvv|csc/i.test(haystacks(f.el).join(" "))) f.cls = "ccCsc";
  }
}

/** Every form in `doc`, with fields outside forms as one more. The kind
 * reads only fields `shown` accepts (hidden honeypots and off-screen
 * fields must not turn a login into a password change). */
export function detect(doc: Document, shown: (el: Field) => boolean = () => true): Detected[] {
  const roots = new Map<HTMLFormElement | Document, Field[]>();
  for (const el of doc.querySelectorAll("input, select")) {
    if (!fillable(el) || el.disabled) continue;
    const root = el.form ?? doc;
    const list = roots.get(root) ?? [];
    list.push(el);
    roots.set(root, list);
  }
  const out: Detected[] = [];
  for (const [root, els] of roots) {
    const fields = els.map((el) => ({ el, cls: classifyField(el) }));
    fixUp(fields);
    const kind = classifyForm(
      fields.filter((f) => shown(f.el)),
      root,
    );
    if (kind) out.push({ root, kind, fields: fields.filter((f) => f.cls !== "unknown") });
  }
  return out;
}
