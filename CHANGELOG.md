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
- Mado Passwords imports password exports from Chrome, Edge, Brave, Safari, Firefox, 1Password and Bitwarden, merging logins that appear more than once, and exports to CSV or to an encrypted .madovault file; profile exports carry their passwords.
- Settings › Passwords manages Mado Passwords: search, details, copying and showing passwords after Touch ID, editing, Recently deleted, codes, health, import and export; mado://passwords opens it, and the welcome sheet asks where you keep passwords.
- When a 1Password or Bitwarden extension is added, Mado can connect that password manager instead, so its own fill menu fills from it; letting the extension fill turns off Mado's fill menu, save prompts and passkeys.
- With AI turned on, Settings › AI connects models: an API key for Anthropic, OpenAI, Google Gemini, xAI, Mistral, DeepSeek, Groq, Cloudflare or any OpenAI-compatible server, signing in with OpenRouter, or Ollama and LM Studio on this Mac. Keys stay in the Keychain, and each connection is tested and shows why it doesn't work.
- Settings › AI picks the model for each job (chat, agent tasks, quick jobs, hard problems, images, page commands) with how hard it thinks and a backup model, or lets Mado choose the best one you have; models can be hidden or starred, with their prices.
- Chat in a side panel about the page you're on, with a screenshot or files if you like: answers stream as they're written, cite their sources when the model searched the web, and chats get a title and are kept for a week (or forever). Bank pages, pages with a password or card field, and Mado's own pages are never sent.
- Ask AI in the page menu, with Text commands turned on: improve, fix, shorten, soften or sharpen what you selected in a text field and put it back in place, or summarize, explain, translate and check code you selected on a page. On macOS 26 and later with Apple Intelligence, the common ones run on your Mac. Apple Intelligence can also be added as a connection.
- Apps on this Mac can control Mado once you turn it on in Settings › Developers and allow each one on a card that says what it may do; you see and revoke them there. The mado command can be given full control, protected settings included, with a notice and an audit entry for every action; passwords and macOS's own prompts still need you.
- The mado command line: open pages, list tabs, read and change settings, run any command by name or by group (mado tab, mado bookmark, mado password…), and see what Mado offers with mado commands and mado guide. It starts Mado hidden when it isn't running and asks you on the Mac the first time.
- Agent tools: AI agents and paired apps can open pages, read them as an accessibility tree, click, type, choose, press keys, hover, scroll and drag with real input, wait for pages to settle, fetch with your cookies, upload and download files, run small scripts of tool calls, sign in without seeing passwords, and change Mado's own tabs and settings with undo. Sensitive pages are never read.
- mado mcp: AI apps that speak the Model Context Protocol, such as Claude Code, can use Mado's agent tools once you allow them in Mado.
- Agents work in their own tabs, grouped under the task and drawn off to the side so your window keeps its tab and focus; showing one of their tabs takes it over until you hand it back. Agents leave your own tabs alone unless Settings › AI allows them.
- Agents ask before paying, sending, deleting, signing in, uploading, changing Mado or leaving the sites you named, showing what they want to do and the total for a payment; a no holds for the rest of the task. Text read on one site can't be typed into another without asking, every step goes in the task's audit, and a spending limit pauses a task.
- Tasks: ask Mado's agent to do something, from the tasks panel or with mado "…". It works step by step in its own tabs, asks you when it needs to, takes steering and follow-ups, can hand parts to helpers, and ends with an answer and its sources. Up to three run at once.
- Subscription CLIs: Claude Code, Codex, Gemini CLI, Copilot CLI and Cursor CLI can run Mado's tasks on the plan you already pay for. Settings › AI finds them and runs their install and login in Terminal, and they reach the browser only through Mado, with the same approvals.

### Fixed

- File inputs on web pages open the file panel again.

<!--
Section order: Added, Changed, Deprecated, Removed, Fixed, Security.
On release, rename Unreleased to [x.y.z] - YYYY-MM-DD and add a link below.
-->

[Unreleased]: https://github.com/tyldra-org/mado/commits/main

- Lists Mado relies on (extension compatibility, banking sites, well-known domains, tracking parameters, HSTS) update daily without an app release, signed so a tampered copy is refused and the last good one stays in use.
