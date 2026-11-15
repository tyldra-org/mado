<script lang="ts">
  // mado://credits: every bundled package and data set with its licence,
  // from the credits.json release builds bundle (scripts/credits.ts).
  import { t } from "../../i18n";

  type Credit = { name: string; version: string; license: string; source: string };
  let credits = $state<Credit[] | null>(null);
  let missing = $state(false);

  $effect(() => {
    fetch("/credits.json")
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((c: Credit[]) => (credits = c))
      .catch(() => (missing = true));
  });
</script>

<main>
  <h1>{t("internal.credits.title")}</h1>
  {#if missing}<p>{t("internal.credits.missing")}</p>{/if}
  {#if credits}
    <table>
      <thead
        ><tr
          ><th>{t("internal.credits.name")}</th><th>{t("internal.credits.version")}</th><th
            >{t("internal.credits.license")}</th
          ></tr
        ></thead
      >
      <tbody>
        {#each credits as c (`${c.name}@${c.version}`)}
          <tr><td>{c.name}</td><td>{c.version}</td><td>{c.license}</td></tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>
