import { describe, expect, test } from "bun:test";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { missing, offered, parseEntry, titlePasses } from "./extension-matrix";

const UBOL = "ddkjiahejlhfcafbddmgiahcphecmpfh";

describe("extension matrix", () => {
  test("a file reads, and a malformed one is refused", () => {
    const e = parseEntry(
      UBOL,
      "name: X\npopup: true\nchecks:\n  - open: /a.html\n    title: ^ok\n",
    );
    expect(e).toEqual({
      id: UBOL,
      name: "X",
      popup: true,
      checks: [{ open: "/a.html", title: "^ok" }],
    });
    expect(() => parseEntry("nope", "name: X")).toThrow();
    expect(() => parseEntry(UBOL, "name: X\nextra: 1")).toThrow();
    expect(() => parseEntry(UBOL, "name: X\nchecks:\n  - open: /a.html\n")).toThrow();
  });

  test("every tested extension needs a file", () => {
    const compat = { extensions: { [UBOL]: { status: "tested" }, b: { status: "mostly" } } };
    expect(missing(compat, [])).toEqual([UBOL]);
    expect(missing(compat, [UBOL])).toEqual([]);
  });

  test("titles pass by pattern or by difference", () => {
    expect(titlePasses({ open: "/", title: "^ad:blocked" }, "ad:blocked app:loaded")).toBe(true);
    expect(titlePasses({ open: "/", titleNot: "rgb(255, 255, 255)" }, "rgb(255, 255, 255)")).toBe(
      false,
    );
  });

  test("the update answer says current or the newer version", () => {
    const xml = (check: string) =>
      `<gupdate><app appid="${UBOL}" status="ok"><updatecheck ${check}/></app></gupdate>`;
    expect(offered(xml('status="noupdate"'), UBOL)).toBeNull();
    expect(offered(xml('codebase="https://x/y.crx" version="2.0" status="ok"'), UBOL)).toBe("2.0");
    expect(() => offered(xml('status="error-unknownApplication"'), UBOL)).toThrow();
  });

  test("the repository's files and the compatibility list agree", async () => {
    const dir = join(import.meta.dir, "../tests/extensions");
    const files = (await readdir(dir)).filter((f) => f.endsWith(".yaml"));
    const ids = await Promise.all(
      files.map(async (f) => parseEntry(f.slice(0, -5), await readFile(join(dir, f), "utf8")).id),
    );
    const compat = JSON.parse(
      await readFile(join(import.meta.dir, "../lists/compat.json"), "utf8"),
    );
    expect(missing(compat, ids)).toEqual([]);
  });
});
