# Security policy

## Supported versions

Mado is pre-release. Security fixes go into the newest release only. Older versions do
not receive fixes; upgrade to receive them.

| Version              | Supported |
| -------------------- | --------- |
| Newest minor release | Yes       |
| Older releases       | No        |

## Reporting a vulnerability

Do not report security vulnerabilities through public issues, discussions or
pull requests.

Report privately through
[GitHub private vulnerability reporting](https://github.com/tyldra-org/mado/security/advisories/new).
If you cannot use GitHub, email yogeshprasad098@gmail.com.

Include as much of the following as you can:

- The affected version or commit.
- The type of issue and its impact.
- Step-by-step reproduction, proof of concept, or exploit code.
- Any known workaround.

## What to expect

- Acknowledgement within 7 days.
- An initial assessment, including severity, within 14 days.
- Progress updates until resolution.
- A fix, a GitHub Security Advisory and, where applicable, a CVE. You are
  credited in the advisory unless you ask not to be.

This is a volunteer-maintained project; these are targets, not guarantees.

## Scope

Mado is a web browser, so the most important reports are about:

- A web page reaching Mado's own interface, commands or internal `mado://` pages.
- A web page or extension reading saved passwords, passkeys, cards or 2FA codes.
- An AI agent acting outside what the user approved, or being steered by text on a web page.
- The local control socket, the `mado` command line or the MCP server accepting a caller it should not.
- Bypassing site permissions, download safety, certificate errors or private windows.
- Tampered updates, extension packages or filter lists being accepted.

Bugs in WebKit or WebView2 themselves belong to Apple or Microsoft; tell us too if Mado makes them reachable.

## Disclosure policy

The project follows coordinated disclosure. Please give maintainers a
reasonable time to release a fix, normally up to 90 days, before public
disclosure. Maintainers will agree a disclosure date with you.
