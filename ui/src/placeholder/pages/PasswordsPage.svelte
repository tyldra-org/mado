<script lang="ts">
  // mado://passwords (spec 11): passwords are managed in Mado's own window,
  // so no secret reaches a page in a tab; this page opens Settings ›
  // Passwords. Placeholder until the design pass (M6).
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";

  let { store }: { store: CoreStore } = $props();
</script>

<main>
  <h1>{t("internal.passwords.title")}</h1>
  <p>{t("internal.passwords.body")}</p>
  <button
    type="button"
    onclick={() =>
      store.client.call({ type: "app.openSettings", section: "passwords" }).catch(() => {})}
    >{t("internal.passwords.open")}</button
  >
</main>
