<script lang="ts">
  // The browser window's UI webview. Placeholder until the design pass (M6):
  // it mirrors the state, reports the page area and shows tabs, the address
  // bar, prompts and notices.
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import Icon from "../design/Icon.svelte";
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
  import SideChat from "./SideChat.svelte";
  import AiResultCard from "./AiResultCard.svelte";
  import PairingCard from "./PairingCard.svelte";
  import ApprovalCard from "./ApprovalCard.svelte";
  import TasksPanel from "./TasksPanel.svelte";
  import PasskeySheet from "./PasskeySheet.svelte";
  import SiteInfo from "./SiteInfo.svelte";
  import TabStrip from "./TabStrip.svelte";
  import UnresponsivePrompt from "./UnresponsivePrompt.svelte";
  import UnsavedPrompt from "./UnsavedPrompt.svelte";
  import NavButtons from "./NavButtons.svelte";

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
  {#if layout === "vertical"}
    <aside class="sidebar" aria-label={t("tabs.label")}>
      <div class="titlebar" data-tauri-drag-region></div>
      <TabStrip {store} {windowId} />
      <div class="sidebar-foot">
        <ProfileMenu {store} />
        <DownloadsBubble {store} />
      </div>
    </aside>
  {/if}
  <div class="content">
    {#if layout === "horizontal"}
      <div class="tabrow" data-tauri-drag-region>
        <TabStrip {store} {windowId} />
      </div>
    {/if}
    <header class="toolbar" data-tauri-drag-region>
      <NavButtons {store} {windowId} />
      <div class="location">
        <SiteInfo {store} {windowId} />
        <AddressBar {store} {windowId} />
      </div>
      <div class="tools">
        <BookmarksBar {store} {windowId} />
        <ExtensionToolbar {store} {windowId} />
        <SideChat {store} {windowId} />
        <TasksPanel {store} open={false} />
        {#if layout === "horizontal"}
          <DownloadsBubble {store} />
          <ProfileMenu {store} />
        {/if}
      </div>
    </header>
    <div id="bookmarks-bar"></div>
    <div id="popovers"></div>
    <FindBar {store} {windowId} />
    <section class="prompts" aria-label={t("window.prompts")}>
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
      <AiResultCard {store} {windowId} />
      <PairingCard {store} />
      <ApprovalCard {store} {windowId} />
      <Notices {store} />
    </section>
    <div class="stage">
      <main bind:this={page} aria-label={t("window.pageArea")}>
        {#if locked}
          <!-- The page is hidden while the private window is locked (08 §6). -->
          <div class="locked" role="dialog" aria-labelledby="lock-title">
            <Icon name="lock" size={28} />
            <h1 id="lock-title">{t("lock.title")}</h1>
            <button type="button" class="primary" onclick={unlock}>{t("lock.unlock")}</button>
            {#if unlockFailed}<p role="alert">{t("lock.failed")}</p>{/if}
          </div>
        {/if}
      </main>
      <div id="side-panels"></div>
    </div>
  </div>
  {#if store.state?.firstRun}<FirstRun {store} />{/if}
</div>

<style>
  .window {
    display: flex;
    height: 100vh;
    background: var(--chrome);
    color: var(--text-on-chrome);
    overflow: hidden;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    width: var(--sidebar-width);
    flex: none;
    min-height: 0;
  }
  .titlebar {
    height: 38px;
    flex: none;
  }
  .sidebar-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-4);
  }
  .content {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    padding: 0 var(--page-gutter) var(--page-gutter) 0;
  }
  .window[data-layout="horizontal"] .content {
    padding-left: var(--page-gutter);
  }
  .tabrow {
    display: flex;
    align-items: center;
    height: 38px;
    padding-left: 76px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-5);
    height: var(--toolbar-height);
    flex: none;
  }
  .location {
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    max-width: 720px;
    margin: 0 auto;
    gap: var(--space-2);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  #popovers {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-4);
  }
  #popovers:not(:empty) {
    padding-bottom: var(--space-4);
  }
  #side-panels {
    display: flex;
    gap: var(--page-gutter);
  }
  #side-panels:empty {
    display: none;
  }
  .prompts {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .prompts:not(:empty) {
    padding-bottom: var(--space-4);
  }
  .stage {
    display: flex;
    flex: 1;
    min-height: 0;
    gap: var(--page-gutter);
  }
  main {
    flex: 1;
    min-width: 0;
    border-radius: var(--radius-card);
    background: var(--surface);
  }
  .locked {
    display: grid;
    justify-items: center;
    align-content: center;
    gap: var(--space-5);
    height: 100%;
    text-align: center;
    color: var(--text);
  }
</style>
