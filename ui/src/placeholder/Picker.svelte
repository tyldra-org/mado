<script lang="ts">
  // The profile picker (spec 01 §1.4). It appears only when several profiles
  // exist, and creating one arrives with profiles (M2-T09), which adds its
  // commands; until then it explains itself.
  import { t } from "../i18n";
</script>

<main>
  <h1>{t("picker.title")}</h1>
  <p>{t("picker.placeholder")}</p>
</main>
