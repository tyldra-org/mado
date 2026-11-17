<script lang="ts">
  // The browser window's UI webview. Placeholder until the design pass (M6):
  // it mirrors the state, reports the page area and shows tabs, the address
  // bar, prompts and notices.
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";
  import AddressBar from "./AddressBar.svelte";
  import AppLinkPrompt from "./AppLinkPrompt.svelte";
  import ExtensionInstallPrompt from "./ExtensionInstallPrompt.svelte";
  import ExtensionToolbar from "./ExtensionToolbar.svelte";
  import OverlapPrompt from "./OverlapPrompt.svelte";
  import BookmarksBar from "./BookmarksBar.svelte";
  import DialogSheet from "./DialogSheet.svelte";
  import DownloadsBubble from "./DownloadsBubble.svelte";
  import FillMenu from "./FillMenu.svelte";
  import FindBar from "./FindBar.svelte";
  import FirstRun from "./FirstRun.svelte";
  import Notices from "./Notices.svelte";
  import PermissionPrompt from "./PermissionPrompt.svelte";
  import ProfileMenu from "./ProfileMenu.svelte";
  import SafariPrompt from "./SafariPrompt.svelte";
  import SavePrompt from "./SavePrompt.svelte";
  import PasskeySheet from "./PasskeySheet.svelte";
  import SiteInfo from "./SiteInfo.svelte";
  import TabStrip from "./TabStrip.svelte";
  import UnresponsivePrompt from "./UnresponsivePrompt.svelte";
  import UnsavedPrompt from "./UnsavedPrompt.svelte";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const settings = $derived(store.state?.settings.values ?? {});
  const theme = $derived(String(settings["appearance.theme"] ?? "system"));
  const layout = $derived(store.state?.windows[windowId]?.layout ?? "vertical");
  const locked = $derived(store.state?.windows[windowId]?.locked ?? false);
  let unlockFailed = $state(false);

  async function unlock() {
    unlockFailed = false;
    try {
      await store.client.call({ type: "window.unlock", windowId });
    } catch {
      unlockFailed = true;
    }
  }

  let page: HTMLElement | undefined = $state();

  // The UI owns layout: report the page rectangle whenever it moves or
  // resizes, at most once per animation frame (spec 01 §5.2).
  $effect(() => {
    const element = page;
    if (!element) return;
    let frame = 0;
    const report = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const r = element.getBoundingClientRect();
        void store.client
          .call({
            type: "window.reportContentRect",
            windowId,
            rect: { x: r.x, y: r.y, width: r.width, height: r.height },
            scale: window.devicePixelRatio,
          })
          .catch(() => {});
      });
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    window.addEventListener("resize", report);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", report);
    };
  });
</script>

<div class="window" data-theme={theme} data-layout={layout}>
  <header>
    <ProfileMenu {store} />
    <TabStrip {store} {windowId} />
    <SiteInfo {store} {windowId} />
    <AddressBar {store} {windowId} />
    <ExtensionToolbar {store} {windowId} />
    <BookmarksBar {store} {windowId} />
    <FindBar {store} {windowId} />
    <DownloadsBubble {store} />
    <UnsavedPrompt {store} {windowId} />
    <UnresponsivePrompt {store} {windowId} />
    <SafariPrompt {store} {windowId} />
    <PermissionPrompt {store} {windowId} />
    <FillMenu {store} {windowId} />
    <SavePrompt {store} {windowId} />
    <PasskeySheet {store} {windowId} />
    <AppLinkPrompt {store} {windowId} />
    <ExtensionInstallPrompt {store} {windowId} />
    <OverlapPrompt {store} />
    <DialogSheet {store} {windowId} />
  </header>
  <main bind:this={page} aria-label={t("window.pageArea")}>
    {#if locked}
      <!-- The page is hidden while the private window is locked (08 §6). -->
      <div class="locked" role="dialog" aria-labelledby="lock-title">
        <h1 id="lock-title">{t("lock.title")}</h1>
        <button type="button" onclick={unlock}>{t("lock.unlock")}</button>
        {#if unlockFailed}<p role="alert">{t("lock.failed")}</p>{/if}
      </div>
    {/if}
  </main>
  {#if store.state?.firstRun}<FirstRun {store} />{/if}
  <Notices {store} />
</div>

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100vh;
    font-family: system-ui, sans-serif;
  }
  .window[data-layout="vertical"] {
    flex-direction: row;
  }
  main {
    flex: 1;
  }
  .locked {
    display: grid;
    place-content: center;
    gap: 1em;
    height: 100%;
    text-align: center;
  }
  .window[data-theme="dark"] {
    color-scheme: dark;
    background: #1e1e1e;
    color: #eee;
  }
</style>
