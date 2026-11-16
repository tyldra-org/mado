# Lists

Data Mado reads that can change without an app release (spec 09 §7, 15 §7). Each file here is built into the app as a fallback. On every push to `main` that changes this folder, `.github/workflows/lists.yml` checks the files with the app's own parsers (`cargo run -p mado-lists --features tool -- check lists`), signs each with the lists minisign key, and publishes the files and their `.minisig` signatures to GitHub Pages at `https://tyldra-org.github.io/mado/lists/`.

Mado fetches them once a day. A list is used only when its signature verifies against the public key built into the app (`crates/mado-lists/src/signature.rs`), names the file it was signed as, is newer than the copy in use, and reads. Otherwise the last good copy, kept in `<root>/Lists/`, stays in use, or the bundled one.

The signature's trusted comment is `timestamp:<serial>\tfile:<name>`, where the serial is the Unix time of the commit the list was signed from. Release builds bundle the lists with the time of their own commit, so a published list older than the app never replaces its copies.

| File | What | Read by | Source |
| --- | --- | --- | --- |
| `compat.json` | Extension statuses and categories, blocked extensions, sites whose notification requests are refused, and the oldest WebKit without pending security fixes (09 §7, 15 §5) | Extensions, About | Maintained here |
| `sensitive-hosts.json` | Banks, whose pages history never indexes and agents never read (12 §2.2, 13 §9); a host covers its subdomains | History, AI | Maintained here |
| `top-domains.txt` | Domains people know, for lookalike checks (05 §4, 08 §2) | Address bar, safety | Chromium `components/url_formatter/spoof_checks/top_domains/domains.list`, BSD-3-Clause, © The Chromium Authors |
| `tracking-params.json` | Parameters Copy clean link removes (08 §8): `prefixes` (any parameter starting with one), `params` (these names on every host), `hosts` (these names on the host and its subdomains only) | Context menus | Maintained here |
| `hsts-preload.txt.gz` | Hosts that must use HTTPS, gzipped lines of `host`, then `+` when subdomains are included (08 §3) | Certificate errors | Chromium `net/http/transport_security_state_static.json`, BSD-3-Clause, © The Chromium Authors |

The filter list manifest arrives with the blocker (M3-T10), payment frames with passwords (M4), the action lexicon and the model catalogue with AI (M5).

To change a list, edit the file and run `cargo nextest run -p mado-lists`, which reads every bundled list.
