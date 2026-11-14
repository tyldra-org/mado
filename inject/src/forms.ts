// `forms`, unsaved-input tracking only (spec 04 §7, §9; the rest of forms
// arrives with Mado Passwords in M4). Mado world, document end, all frames.
import { sender } from "./lib/bridge";

const send = sender("forms");
let dirty = false;

const set = (value: boolean) => {
  if (value === dirty) return;
  dirty = value;
  send("unsavedInput", { dirty });
};

const counts = (el: EventTarget | null): boolean => {
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    const type = el.type.toLowerCase();
    return (
      ["text", "email", "url", "tel", "number", ""].includes(type) &&
      el.getAttribute("role") !== "searchbox"
    );
  }
  return el instanceof HTMLElement && el.isContentEditable;
};

document.addEventListener(
  "input",
  (e) => {
    if (counts(e.target)) set(true);
  },
  { capture: true, passive: true },
);
document.addEventListener("submit", () => set(false), { capture: true, passive: true });
addEventListener("pagehide", () => set(false));
