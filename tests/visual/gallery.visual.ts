// Every gallery screen in light and dark against its stored image (M6-T10).
import { expect, test } from "@playwright/test";

type Shot = { name: string; query: string; size: [number, number] };

const window: [number, number] = [1180, 780];
const shots: Shot[] = [];
for (const theme of ["light", "dark"]) {
  shots.push(
    { name: `window-vertical-${theme}`, query: `screen=window&theme=${theme}`, size: window },
    {
      name: `window-horizontal-${theme}`,
      query: `screen=window&layout=horizontal&theme=${theme}`,
      size: window,
    },
    {
      name: `window-compact-${theme}`,
      query: `screen=window&density=compact&theme=${theme}`,
      size: window,
    },
    {
      name: `approval-${theme}`,
      query: `screen=window&prompt=approval&theme=${theme}`,
      size: window,
    },
    {
      name: `permission-${theme}`,
      query: `screen=window&prompt=permission&theme=${theme}`,
      size: window,
    },
    {
      name: `save-password-${theme}`,
      query: `screen=window&prompt=save&theme=${theme}`,
      size: window,
    },
    {
      name: `settings-appearance-${theme}`,
      query: `screen=settings&section=appearance&theme=${theme}`,
      size: [880, 620],
    },
    {
      name: `settings-ai-${theme}`,
      query: `screen=settings&section=ai&theme=${theme}`,
      size: [880, 620],
    },
    {
      name: `newtab-${theme}`,
      query: `screen=internal&page=newtab&theme=${theme}`,
      size: [1032, 744],
    },
    {
      name: `history-${theme}`,
      query: `screen=internal&page=history&theme=${theme}`,
      size: [1032, 744],
    },
  );
}

for (const shot of shots) {
  test(shot.name, async ({ page }) => {
    await page.setViewportSize({ width: shot.size[0], height: shot.size[1] });
    // The gallery's stand-in page loads in a frame; wait for it too.
    await page.goto(`/gallery.html?${shot.query}`, { waitUntil: "networkidle" });
    await page.waitForTimeout(300);
    await expect(page).toHaveScreenshot(`${shot.name}.png`);
  });
}
