<script lang="ts">
  // Mado's own pages in tabs, `mado://<page>` (spec 01 §4.4). They talk to
  // the core through the `internal` page script, within their allow-list.
  // Placeholders until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";
  import AboutPage from "./pages/AboutPage.svelte";
  import BookmarksPage from "./pages/BookmarksPage.svelte";
  import CreditsPage from "./pages/CreditsPage.svelte";
  import DownloadsPage from "./pages/DownloadsPage.svelte";
  import HistoryPage from "./pages/HistoryPage.svelte";
  import NewTabPage from "./pages/NewTabPage.svelte";

  let { store, page }: { store: CoreStore; page: string } = $props();

  const titles: Record<string, string> = {
    newtab: "internal.newtab.title",
    history: "internal.history.title",
    downloads: "internal.downloads.title",
    bookmarks: "internal.bookmarks.title",
    about: "internal.about.title",
    credits: "internal.credits.title",
  };

  $effect(() => {
    document.title = t(titles[page] ?? "internal.notFound");
  });
</script>

{#if page === "newtab"}
  <NewTabPage {store} />
{:else if page === "history"}
  <HistoryPage {store} />
{:else if page === "downloads"}
  <DownloadsPage {store} />
{:else if page === "bookmarks"}
  <BookmarksPage {store} />
{:else if page === "about"}
  <AboutPage {store} />
{:else if page === "credits"}
  <CreditsPage />
{:else}
  <main><h1>{t("internal.notFound")}</h1></main>
{/if}
