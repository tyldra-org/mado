<script lang="ts">
  // The platform test page of development builds (M1-T10): one button per
  // mado-platform function, for checks that need a person. Not translated:
  // it never ships to users.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  type Row = { label: string; action: string; arg?: string };
  const permissions = ["camera", "microphone", "location", "screen", "speech"];
  const rows: Row[] = [
    { label: "Paths", action: "paths" },
    { label: "Keychain: write", action: "keychain.set", arg: "secret-value" },
    { label: "Keychain: read", action: "keychain.get" },
    { label: "Keychain: delete", action: "keychain.delete" },
    { label: "Touch ID available", action: "auth.available" },
    { label: "Authenticate (Touch ID or password)", action: "auth.authenticate" },
    { label: "Notifications: ask permission", action: "notify.request" },
    { label: "Notifications: post (click it)", action: "notify.post", arg: "Click me" },
    ...permissions.flatMap((p) => [
      { label: `${p}: status`, action: "permission.status", arg: p },
      { label: `${p}: request`, action: "permission.request", arg: p },
    ]),
    { label: "Open at login: status", action: "login.enabled" },
    { label: "Open at login: on", action: "login.set", arg: "on" },
    { label: "Open at login: off", action: "login.set", arg: "off" },
    { label: "Default browser: is Mado", action: "defaultBrowser.is" },
    { label: "Default browser: make Mado", action: "defaultBrowser.make" },
    { label: "Dock badge: set W", action: "dock.badge", arg: "W" },
    { label: "Dock badge: clear", action: "dock.badge", arg: "" },
  ];

  let results: Record<string, string> = $state({});
  let clicked: string[] = $state([]);

  $effect(() => {
    const off = listen<string>("dev://notification-clicked", (e) => {
      clicked = [...clicked, e.payload];
    });
    return () => void off.then((f) => f());
  });

  async function run(row: Row) {
    const key = row.label;
    results[key] = "…";
    try {
      results[key] = JSON.stringify(
        await invoke("mado_dev_platform", { action: row.action, arg: row.arg ?? "" }),
      );
    } catch (e) {
      results[key] = `error: ${String(e)}`;
    }
  }
</script>

<main>
  <h1>Platform test page</h1>
  <table>
    <tbody>
      {#each rows as row (row.label)}
        <tr>
          <td><button type="button" onclick={() => run(row)}>{row.label}</button></td>
          <td><output>{results[row.label] ?? ""}</output></td>
        </tr>
      {/each}
    </tbody>
  </table>
  <p>Notification clicks: <output>{clicked.join(", ") || "none"}</output></p>
</main>
