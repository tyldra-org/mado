// The keyboard pass (roadmap M6-T09): ⌥Tab reaches every visible
// control of the window and the settings window, in order, and the focus
// ring shows. Tab rows use arrow keys (one tab stop for the strip), so rows
// other than the active one are not Tab stops.
import { expect, test } from "@playwright/test";

const screens = [
  { name: "window", query: "screen=window&theme=light", size: [1180, 780] },
  {
    name: "window-horizontal",
    query: "screen=window&layout=horizontal&theme=dark",
    size: [1180, 780],
  },
  { name: "settings", query: "screen=settings&section=appearance&theme=light", size: [880, 620] },
] as const;

for (const s of screens) {
  test(`Tab reaches every control: ${s.name}`, async ({ page }) => {
    await page.setViewportSize({ width: s.size[0], height: s.size[1] });
    await page.goto(`/gallery.html?${s.query}`, { waitUntil: "networkidle" });
    const wanted: string[] = await page.evaluate(() => {
      const all = [
        ...document.querySelectorAll<HTMLElement>(
          "button:not([disabled]), input:not([disabled]), select, textarea, a[href], [tabindex='0']",
        ),
      ];
      return all
        .filter((e) => e.tabIndex >= 0 && e.offsetParent !== null && e.getClientRects().length > 0)
        .map((e, i) => {
          e.dataset.kbd = String(i);
          return String(i);
        });
    });
    expect(wanted.length).toBeGreaterThan(5);
    const reached = new Set<string>();
    let ringless = 0;
    for (let i = 0; i < wanted.length * 3 + 10; i++) {
      // WebKit moves Tab between fields only; ⌥Tab reaches every control,
      // as Safari and macOS's keyboard navigation setting do.
      await page.keyboard.press("Alt+Tab");
      const info = await page.evaluate(() => {
        const a = document.activeElement as HTMLElement | null;
        if (!a?.dataset.kbd) return null;
        const style = getComputedStyle(a);
        return {
          id: a.dataset.kbd,
          // The address field draws its ring on its row.
          ring:
            style.outlineStyle !== "none" ||
            style.boxShadow !== "none" ||
            getComputedStyle(a.parentElement as HTMLElement).boxShadow !== "none",
        };
      });
      if (info) {
        reached.add(info.id);
        if (!info.ring) ringless++;
      }
    }
    const missed = wanted.filter((w) => !reached.has(w));
    const names = await page.evaluate(
      (ids) =>
        ids.map((id) => {
          const e = document.querySelector<HTMLElement>(`[data-kbd="${id}"]`);
          return `${e?.tagName} ${e?.getAttribute("aria-label") ?? e?.textContent?.trim().slice(0, 30)}`;
        }),
      missed,
    );
    expect(names).toEqual([]);
    expect(ringless).toBe(0);
  });
}
