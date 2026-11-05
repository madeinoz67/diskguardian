# Security Policy

DiskGuardian is **pre-1.0** software, MIT-licensed, maintained by one person. This policy
is written to be honest about that rather than to promise more than it can deliver.

## Reporting a vulnerability

**Please report privately, not in a public issue.**

Use GitHub's private vulnerability reporting:
[**Report a vulnerability**](https://github.com/madeinoz67/diskguardian/security/advisories/new).
It creates a private advisory only you and the maintainer can see, and it handles
coordinated disclosure and CVE requests if it gets that far.

If you can, include:

- The version or commit you tested (`DiskGuardian --version`, or the build you installed)
- Which surface is affected — the scan engine, the Tauri shell, the sunburst frontend, the
  administrator measurement helper, or the Windows elevation path
- What an attacker gains, and what access they need to start (local files they control? a
  folder they can create? an already-elevated process?)
- The smallest reproduction you can manage

Reports are acknowledged and worked on a **best-effort** basis. No response time is
promised that cannot be honored. If something is being actively exploited, say so in the
report and it will be treated accordingly.

Please give a reasonable chance to ship a fix before disclosing publicly. Credit in the
advisory is gladly given — say how you want to be credited, or that you would rather not
be.

## Scope

**Supported version: the latest release.** DiskGuardian is pre-1.0 and fixes are not
backported to older tags. The previous minor is not maintained.

The whole product is one promise: it shows you what is really on the disk, and it deletes
only what you asked it to. In scope is anything that breaks either half of that.

In scope:

- **Anything that makes the app delete something it should have refused** — a system
  tree, the scan root itself, a path that was never in the scan it holds in memory, or a
  deletion that skips the confirmation above 5 GB
- **Anything that makes it delete outside the scanned tree** — a crafted folder name,
  a symlink swapped between the scan and the delete, or a path the frontend can inject
  into a command
- **Anything the privileged measurement helper can do besides measure.** It is supposed
  to walk restricted paths read-only and write numbers back; it contains no deletion code.
  Escaping its job list, writing files, or outliving the app are all in scope
- **The Windows elevation path** — an elevated instance that outlives its parent, that
  measures more than the pass asked for, or that can be triggered without the user
  pressing *Measure restricted areas…*
- **The IPC boundary** — the webview reaching a command it should not, or corrupting the
  favourites store
- **Secrets or personal data leaking** — file names, paths or scan contents arriving in
  logs, panic messages, error text, crash reports, or any network call. The app makes no
  network calls; if you find one, that is a finding
- **Untrusted input reaching memory unsafety or code execution** — filesystem data,
  filenames, bundle files, or a malformed `favourites.json`

Out of scope:

- **What you choose to delete.** The tool deletes, guarded and with confirmation; a file
  you selected and confirmed is the product working, not a vulnerability.
- **The builds are unsigned.** macOS quarantines the download and Windows SmartScreen
  warns — both documented in the README, both expected until there is a signing identity.
  "The app is not code-signed" is not a finding; a way to make the app run code it
  should not is.
- **Restricted areas still shown as restricted.** Root does not bypass macOS privacy
  controls (TCC); folders like `~/Library/Mail` stay restricted until Full Disk Access is
  granted. That is the documented behaviour, not a defect.
- **Anything requiring filesystem or OS-level access the attacker already has.**
  DiskGuardian is a local desktop application and does not defend against a compromised
  machine.
- **A scan that is slow or memory-hungry on a pathological tree.** That is a performance
  bug worth a public issue, not a security report.
- **Findings from automated scanners with no demonstrated impact.** If you cannot say
  what an attacker gains, it belongs in an issue rather than an advisory.

## Known weaknesses

Tracked openly rather than defended:

- No code signing on any platform, so no artifact can prove where it came from.
- Restricted-area measurement is not implemented on Linux; those areas stay shown as
  restricted.
- The administrator measurement helper runs with full privileges. It is written to
  measure and nothing else, and it dies with the app — but its privileges are real, and
  a defect in it is a serious finding rather than a low one.

If you find something already tracked as a public issue, a comment there is more useful
than a new report — but if you think it is more severe than it was rated, say so
privately. Re-rating severity beats defending it.
