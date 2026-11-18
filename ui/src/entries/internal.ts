// Mado's own pages in tabs (spec 01 §4.4): `mado://<page>` loads this
// entry; the page is the URL's host.
import "../design/tokens.css";
import "../design/base.css";
import { mount } from "svelte";
import { CoreStore } from "../core/store.svelte";
import { pageTransport } from "../core/transport";
import { followTheme } from "../design/theme.svelte";
import Internal from "../placeholder/Internal.svelte";

const target = document.getElementById("app");
if (target) {
  const store = new CoreStore(pageTransport());
  document.documentElement.dataset.surface = "internal";
  followTheme(store);
  mount(Internal, { target, props: { store, page: location.host } });
  void store.start();
}
