// Spec 11 §3 and roadmap M4-T02: the detector on the fixture pages in
// tests/forms/ classifies at least 95% of fields and every form right.
import { describe, expect, test } from "bun:test";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { JSDOM } from "jsdom";
import { detect, type FormKind } from "./formdetect";

const dir = join(import.meta.dir, "../../../tests/forms");

function load(html: string): Document {
  const dom = new JSDOM(html);
  const w = dom.window as unknown as Record<string, unknown>;
  for (const name of [
    "HTMLInputElement",
    "HTMLSelectElement",
    "HTMLTextAreaElement",
    "HTMLFormElement",
    "Document",
  ]) {
    (globalThis as Record<string, unknown>)[name] = w[name];
  }
  return dom.window.document;
}

describe("form detection", async () => {
  const pages = (await readdir(dir)).filter((f) => f.endsWith(".html"));
  let right = 0;
  let total = 0;
  const wrong: string[] = [];

  for (const page of pages) {
    test(page, async () => {
      const doc = load(await readFile(join(dir, page), "utf8"));
      const found = detect(doc);
      const classOf = new Map<Element, string>();
      for (const f of found) for (const field of f.fields) classOf.set(field.el, field.cls);
      for (const el of doc.querySelectorAll("[data-expect]")) {
        total += 1;
        const want = el.getAttribute("data-expect");
        const got = classOf.get(el) ?? "unknown";
        if (got === want) right += 1;
        else wrong.push(`${page}: ${el.getAttribute("name") ?? el.id}: ${got}, not ${want}`);
      }
      const kinds = [...doc.querySelectorAll("[data-expect-kind]")].map((el) => ({
        root: el.tagName === "FORM" ? el : doc,
        kind: el.getAttribute("data-expect-kind") as FormKind | null,
      }));
      for (const { root, kind } of kinds) {
        const got: string | null = found.find((f) => f.root === root)?.kind ?? null;
        expect(got, `${page} form kind`).toBe(kind as string | null);
      }
      expect(found.length, `${page}: no other forms`).toBe(kinds.length);
    });
  }

  test("at least 95% of fields are classified right", () => {
    expect(total).toBeGreaterThan(60);
    if (wrong.length > 0) console.log(wrong.join("\n"));
    expect(right / total).toBeGreaterThanOrEqual(0.95);
  });
});
