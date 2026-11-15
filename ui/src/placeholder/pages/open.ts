// Opening a link from a `mado://` page: the core acts from the page's own
// tab, so the IDs only need to be well formed.
import type { CoreStore } from "../../core/store.svelte";

export async function openUrl(store: CoreStore, url: string, background = false): Promise<void> {
  const ids = (await store.client.call({ type: "state.get" })) as {
    tabId: string;
    windowId: string;
  };
  await store.client
    .call({
      type: "nav.open",
      windowId: ids.windowId,
      tabId: ids.tabId,
      input: url,
      disposition: background ? "newBackgroundTab" : "currentTab",
      source: "bookmark",
      ctrlEnter: false,
    })
    .catch(() => {});
}
