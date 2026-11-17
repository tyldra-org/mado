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
- Profiles can be exported to a `.madoprofile` file and imported on another Mac, without page text or session history.
- Private and guest windows run in a temporary profile with nothing kept on disk, deleted when they close.
- Safety: a first visit to a site that imitates a well-known or frequently visited one asks "Did you mean …?", and a site with an untrusted certificate shows a warning you can proceed past for the session, except on sites that require secure connections.
- Privacy: Clear browsing data by time range (history, downloads list, cookies and site data, cache, site settings, form entries), Clear on quit, and Clear cookies and site data for one site from the site-information panel. Global Privacy Control, strict tracking prevention (no third-party cookies), hiding your local IP address from WebRTC and the dangerous-site warning follow their settings in open tabs too. Copy clean link also removes `si` from YouTube and Spotify links.
- Full screen: a page that goes full screen shows "<site> is now full screen · Press Esc to exit", again when the pointer reaches the top, and Esc always exits. View certificate on certificate warnings and in the site-information panel. Private windows can lock after five minutes in the background until you unlock them with Touch ID, and start with your settings.
- Mado's own pages open in tabs: New Tab, History, Downloads, Bookmarks, About and Credits, each able to do only what it needs.
- A welcome sheet on first launch: import from another browser, choose a search engine (none is preselected) and whether to show suggestions, and make Mado the default browser, which Settings › General also offers.
- Updates: Relaunch to update brings every open profile back with its windows and tabs; security updates install by themselves once nobody has used Mado for 10 minutes. About shows Mado's and WebKit's versions and warns when macOS has web engine security fixes waiting.
- Menus with Chrome's structure and shortcuts, a Dock menu for new windows, F6 and ⌘L to reach the address bar, and a find bar. Every shortcut can be changed in Settings › Keyboard shortcuts, which warns when keys are taken; browser shortcuts like ⌘T and ⌘W always work, even on pages that grab keys.
- Import from Chrome, Edge, Brave, Arc, Firefox and Safari: bookmarks, history (the last 90 days or all of it), search engines, open tabs with their pins and groups, Arc's spaces as saved groups, and the list of extensions to reinstall later. Importing again adds nothing twice. Safari asks for Full Disk Access and Mado shows how to grant it.
- Reader: pages that look like articles offer Reader, which shows the text, images and tables only, in your choice of font, size, width and colours; Back leaves it, and a site can always open in Reader.
- Downloads: files go to the Downloads folder under a unique name, can be paused and resumed (also after quitting Mado), get the quarantine mark, and wait for Keep when they are insecure or a risky type from a new site. A downloads button lists them.
- History: every visit is recorded with how it happened, pages' text is searchable (never on banking sites or pages with password fields), and history can be deleted by visit, site or time, excluded per site, and exported as CSV.
- Tabs, windows and groups: opening, closing and selecting tabs as Chrome does, pinned tabs, saved groups, ⌘⇧T for closed tabs, groups and windows, sleeping tabs with Memory Saver, startup modes with crash restore, and a warning before quitting with typed text.
- Nightly builds for Apple silicon Macs, signed with Mado's own certificate, that update themselves on the nightly channel. Intel Macs are not supported.
- Lists Mado relies on (extension compatibility, banking sites, well-known domains, tracking parameters, HSTS) update daily without an app release, signed so a tampered copy is refused and the last good one stays in use.
- Extensions from the Chrome Web Store: "Add to Mado" on a store page downloads and verifies the extension and asks before adding it, showing what it can read and use and what Mado doesn't support yet. In Developer mode, extensions also install from a folder, a `.crx` or a `.zip`.
- The built-in blocker's filter lists (uBlock Origin's, EasyList, EasyPrivacy, Peter Lowe's) download daily, keep their last good copy when a download is broken, and a bundled EasyList blocks from the first launch, offline too.
- The built-in blocker blocks ads and trackers in pages and hides ad boxes, and counts what it blocked on each tab.
- Extension buttons in the toolbar open their popups; pin them, open their options, and manage extensions in Settings › Extensions: turn them on or off, choose where they run, grant optional permissions, set shortcuts, remove them with or without their data, and in Developer mode load one from a folder, see its errors and inspect it.
- Extensions can talk to apps on the Mac through native messaging, using the hosts apps register for Chrome, so password manager extensions can reach their desktop apps.
- The blocker hides page elements its lists name, including by their text, runs uBlock Origin's scriptlets where filters call for them, and lets you pick an element on a page to hide it there from then on, with Undo.
- Extensions from the Chrome Web Store update daily; an update that asks for more stays off until you review it, and extensions Mado learns are harmful are turned off.
- When an extension does what Mado already does (blocking ads, filling passwords, putting tabs to sleep), Mado asks which should do it, and gives its settings back when the extension goes. Extensions that change the new tab page, search engine or startup pages ask first, and the settings they control say so.
- The blocker counts what it blocks: the site-info panel shows a page's count and turns the blocker off for a site, Settings › Privacy chooses filter lists, takes custom filters and shows a privacy report of the ads and trackers blocked, and a page reloaded three times after blocks offers to turn the blocker off there.
- Extensions found when importing from another browser are offered in Settings › Extensions, and the store's button reads "Add to Mado", "Remove from Mado" or "Not available in Mado" without Chrome's label left beside it. An extension Mado doesn't support yet says so in its install prompt.
- Mado Passwords has its vault: items are encrypted with a key that the Mac's Secure Enclave wraps and Touch ID unlocks, a recovery secret opens it on another Mac, it locks with the screen and after 15 minutes without use, and deleted items stay in Recently deleted for 30 days.
- Mado Passwords fills sign-in, card, address and contact forms from a menu at the field you click: only visible fields in the page itself (or a payment provider's frame for cards), the password page of a two-step sign-in is offered the same login, and a saved two-factor code fills itself after signing in.
- Mado Passwords offers to save a password after a sign-in that worked, or to update one that changed, with the username and password editable and "Never on this site"; sign-up and change-password forms offer a strong password that follows the site's rules and saves itself.
- 1Password and Bitwarden join Mado Passwords as places passwords come from and go to: turn each on in Settings › Passwords (1Password through its CLI integration, Bitwarden unlocked with its master password once), and the fill menu shows items from all of them, a login saved in several places once.
- Mado Passwords reads a two-factor QR code shown on a page and adds the code to that site's login, or as a code of its own.
- Mado Passwords creates and uses passkeys: sites that offer passkeys get a Mado sheet to save one (Touch ID), sign in by choosing an account or from the fill menu, and "Use another device" hands the request back to macOS.
- Mado Passwords checks password health: passwords reused across sites, weak ones, and, only when you ask, ones found in breaches (Have I Been Pwned, sending just five characters of each password's hash).

<!--
Section order: Added, Changed, Deprecated, Removed, Fixed, Security.
On release, rename Unreleased to [x.y.z] - YYYY-MM-DD and add a link below.
-->

[Unreleased]: https://github.com/tyldra-org/mado/commits/main

- Lists Mado relies on (extension compatibility, banking sites, well-known domains, tracking parameters, HSTS) update daily without an app release, signed so a tampered copy is refused and the last good one stays in use.
