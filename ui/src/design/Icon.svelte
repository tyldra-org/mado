<script lang="ts">
  // One of Mado's line icons. Decorative unless `label` is given.
  import { type IconName, icons } from "./icons";

  let { name, size = 16, label }: { name: IconName; size?: number; label?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <path d={icons[name]} />
</svg>

<style>
  svg {
    flex: none;
    display: block;
  }
</style>
