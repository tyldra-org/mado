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
- History: every visit is recorded with how it happened, pages' text is searchable (never on banking sites or pages with password fields), and history can be deleted by visit, site or time, excluded per site, and exported as CSV.
- Tabs, windows and groups: opening, closing and selecting tabs as Chrome does, pinned tabs, saved groups, ⌘⇧T for closed tabs, groups and windows, sleeping tabs with Memory Saver, startup modes with crash restore, and a warning before quitting with typed text.
- Nightly builds for Apple silicon Macs, signed with Mado's own certificate, that update themselves on the nightly channel. Intel Macs are not supported.

<!--
Section order: Added, Changed, Deprecated, Removed, Fixed, Security.
On release, rename Unreleased to [x.y.z] - YYYY-MM-DD and add a link below.
-->

[Unreleased]: https://github.com/tyldra-org/mado/commits/main
