// Bookmarks through the app's commands (spec 12 §3, roadmap M2-T11).
import { afterAll, describe, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { bin, Mado } from "./harness";

const run = bin ? describe : describe.skip;

type Node = {
  kind: string;
  title: string;
  url: string | null;
  root: string | null;
  children?: Node[];
};
const shape = (nodes: Node[]): unknown =>
  nodes.map((n) => ({
    kind: n.kind,
    title: n.title,
    url: n.url,
    children: n.children ? shape(n.children) : undefined,
  }));

run("bookmarks", () => {
  const started: Mado[] = [];
  afterAll(async () => {
    for (const m of started) {
      await m.quit();
      await m.remove();
    }
  });

  async function fresh(): Promise<Mado> {
    const m = await Mado.create();
    await m.start();
    started.push(m);
    return m;
  }

  test("export then import into an empty profile gives an identical tree (12 §7 check 4)", async () => {
    const a = await fresh();
    const [bar] = await a.control.call<Node[] & { id: string }[]>("bookmark.tree", {});
    const barId = (bar as unknown as { id: string }).id;
    await a.control.call("bookmark.add", {
      parentId: barId,
      title: "News & more",
      url: "https://news.example/?a=1&b=2",
    });
    const { id: folder } = await a.control.call<{ id: string }>("bookmark.addFolder", {
      parentId: barId,
      title: "Work",
    });
    await a.control.call("bookmark.add", {
      parentId: folder,
      title: "Docs",
      url: "https://docs.example/",
    });
    await a.control.call("bookmark.add", { title: "Elsewhere", url: "https://other.example/" });
    const file = join(tmpdir(), `mado-bookmarks-${Date.now()}.html`);
    await a.control.call("bookmark.export", { path: file });

    const b = await fresh();
    const { imported } = await b.control.call<{ imported: number }>("bookmark.import", {
      path: file,
    });
    expect(imported).toBe(3);
    const treeA = await a.control.call<Node[]>("bookmark.tree", {});
    const treeB = await b.control.call<Node[]>("bookmark.tree", {});
    expect(shape(treeB)).toEqual(shape(treeA));
  });

  test("deleting offers Undo, which puts the bookmark back", async () => {
    const m = await fresh();
    const { id } = await m.control.call<{ id: string }>("bookmark.add", {
      title: "Gone",
      url: "https://gone.example/",
    });
    await m.control.call("bookmark.delete", { ids: [id] });
    const after = await m.control.call<unknown[]>("bookmark.ofUrl", {
      url: "https://gone.example/",
    });
    expect(after).toEqual([]);
    const s = await m.control.call<{
      state: { notices: { undo: { type: string; token: number } | null }[] };
    }>("state.get");
    const undo = s.state.notices.find((n) => n.undo?.type === "bookmark.undoDelete")?.undo;
    expect(undo).toBeDefined();
    await m.control.call("bookmark.undoDelete", { token: undo?.token });
    const back = await m.control.call<unknown[]>("bookmark.ofUrl", {
      url: "https://gone.example/",
    });
    expect(back.length).toBe(1);
  });
});
