<script lang="ts">
  // The browser window's UI webview. Placeholder until the design pass (M6):
  // it mirrors the state, reports the page area and shows notices. Tabs, the
  // sidebar and the address bar arrive in M2.
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";
  import Notices from "./Notices.svelte";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const settings = $derived(store.state?.settings.values ?? {});
  const theme = $derived(String(settings["appearance.theme"] ?? "system"));
  const layout = $derived(store.state?.windows[windowId]?.layout ?? "vertical");

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
    <h1>{store.state?.profile.name ?? t("app.name")}</h1>
    <p>{t("window.layout", { layout })}</p>
  </header>
  <main bind:this={page} aria-label={t("window.pageArea")}>
    <p>{t("app.placeholder")}</p>
  </main>
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
  .window[data-theme="dark"] {
    color-scheme: dark;
    background: #1e1e1e;
    color: #eee;
  }
</style>
