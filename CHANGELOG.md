# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Repository scaffold: Rust workspace with one crate per area, Svelte UI and page script packages, checks and CI.
- The app starts: one process per profile with a profile lock, a second launch hands over to the first, and Cmd-Q or SIGTERM quits cleanly.
- Settings: every setting of the design, stored per app or profile, with protection, undo, search, export and import, and a settings window.
- Storage: SQLite databases that refuse a newer schema, back up before migrating and restore from a backup when damaged.
- Web pages load in tabs through WebKit, each profile with its own website data and proxy, and sites see Mado as Safari.
- Find in page, mute, link previews in the status bubble, page icons, a prompt for pages that stop responding, and reload, back, forward, zoom, print and save for tabs.
- Bookmarks: a bookmarks bar and Other bookmarks, ⌘D to add or edit, folders that open as tabs, Undo after deleting, and import and export in the HTML format every browser reads.
- Address bar: type a web address, a search, a search engine's keyword or a calculation (arithmetic, units, time zones, dates). Suggestions come from open tabs, history, bookmarks and page text, with inline completion and, if you turn them on, your search engine's suggestions. Eight search engines to choose from, and sites' own search engines are offered.
- Addresses show the site's domain, and internationalised domains that imitate other sites stay in their `xn--` form.
- Sites load over HTTPS when they can; for sites without it Mado asks before continuing. Pages that fail to load explain why, offer to sign in to the network behind a captive portal, and show when a page crashed.
- ⌘-click, ⌘⇧-click and ⇧-click open links in a new tab, a new foreground tab or a new window; ⌥-click downloads. Sites whose videos only play in Safari can open there, once or always.
- Site permissions: sites ask before using the camera, microphone, location, notifications and more, with Allow while visiting, Allow this time and Never; prompts can't be clicked in their first half second, sites that keep asking get blocked, and permissions unused for 60 days are removed with Undo. Page notifications appear in macOS.
- Links to other apps (Zoom, FaceTime and others) ask first, showing the app's name and icon, and can be allowed for a site; dangerous schemes never open and a page can't open apps in a flood. Sites can offer to handle mail links.
- Page dialogs (alerts, confirmations, questions, "Leave site?") show over their tab with the site's name, a page that keeps showing them can be stopped, and hidden frames from other sites show none. Sites can ask you to sign in or for a certificate from your keychain, and sending a form from a secure page over an insecure connection asks first.
- Site information next to the address bar shows a site's connection, what it may use and what it is using now, with Stop; Site settings lists every site with its own settings. A checkout asking for Apple Pay offers to open in Safari, where it works.
- Context menus on pages, for links, images, video, selected text, text fields and the page itself, including Copy Clean Link (without tracking parameters), Copy Link as Markdown and Copy Link to Highlight.
- Profiles: add, rename, recolour, open and delete profiles from the profile menu and Settings, each in its own window and process; deleting one removes its data, website data and Keychain items. Rules such as `*.acme.com` open links from other apps in the profile you choose.
- Downloads: files go to the Downloads folder under a unique name, can be paused and resumed (also after quitting Mado), get the quarantine mark, and wait for Keep when they are insecure or a risky type from a new site. A downloads button lists them.
- History: every visit is recorded with how it happened, pages' text is searchable (never on banking sites or pages with password fields), and history can be deleted by visit, site or time, excluded per site, and exported as CSV.
- Tabs, windows and groups: opening, closing and selecting tabs as Chrome does, pinned tabs, saved groups, ⌘⇧T for closed tabs, groups and windows, sleeping tabs with Memory Saver, startup modes with crash restore, and a warning before quitting with typed text.
- Nightly builds for Apple silicon Macs, signed with Mado's own certificate, that update themselves on the nightly channel. Intel Macs are not supported.

<!--
Section order: Added, Changed, Deprecated, Removed, Fixed, Security.
On release, rename Unreleased to [x.y.z] - YYYY-MM-DD and add a link below.
-->

[Unreleased]: https://github.com/tyldra-org/mado/commits/main
