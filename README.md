<p align="center">
  <img src="branding/logos/overlook-social-preview.png" alt="Overlook — A clear view of what matters." width="820">
</p>

<p align="center">
  <strong>A calmer place for your mail.</strong> A native Gmail and Google Calendar client for Linux, written in Rust.<br>
  Thoughtfully made for Linux, Arch, and Omarchy. By <a href="https://lastrefuge.ai/">Last Refuge</a>.
</p>

<p align="center">
  <a href="#install"><img alt="Install with one command" src="https://img.shields.io/badge/install-one%20command-365F73?style=flat-square"></a>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.99-536E63?style=flat-square&logo=rust&logoColor=white">
  <img alt="GPUI" src="https://img.shields.io/badge/UI-GPUI-365F73?style=flat-square">
  <img alt="Linux" src="https://img.shields.io/badge/Linux-Wayland-101C24?style=flat-square&logo=linux&logoColor=white">
  <img alt="Status" src="https://img.shields.io/badge/status-early%20daily%20use-FF9D32?style=flat-square">
</p>

---

Overlook is for people who liked the way **New Outlook** works and want that on Linux: a spacious three-pane
workspace, a Focused Inbox, a real calendar, and a composer that feels finished. It is **local-first**: your
mail and events live in a database on your machine, every change is saved on this device before it is sent,
and you can read, search, write, and file mail with no connection at all.

<p align="center">
  <img src="docs/screenshots/inbox-light.png" alt="Overlook inbox in the Light theme" width="920">
</p>

<p align="center"><sub>The inbox in Light. Fictional sample mail, included with the app.</sub></p>

## What you get

- **Mail the Outlook way.** Account and folder sidebar, message list with day groups, a generous reading
  pane you can move below the list, comfortable and compact density, resizable panes that remember their size.
- **Focused Inbox.** The Inbox splits into Focused and Other from Gmail's own hints, who you write to, and
  your choices: *Move to Other*, or *Always move messages from this sender*. Local, private, and reversible.
- **Reading that respects you.** Received HTML is sanitized before it is rendered; scripts never run and remote
  images are blocked until you ask. Attachments download on click and open with your system apps.
- **Writing.** New, reply, reply all, forward, with autosave to the device, recipient suggestions from people
  you have written to, attachments, and signatures per account. Formatting as you type: bold, italics, links,
  lists, quotes and code, styled live in the editor with the marks still visible, a Preview of exactly what
  recipients get, and the quoted original kept out of your way below the reply.
- **A calendar, not an afterthought.** Day, Week, Month, and Agenda. Invitations with Accept, Tentative, and
  Decline, from the calendar or from the email itself. Create, edit, and delete events. Reminders as desktop
  notifications. Working hours shaded. Google's event colors.
- **Offline, properly.** Archive, star, move, reply, and send while disconnected; everything is queued in order
  and replayed when you are back. A visible Outbox says *Waiting for connection*; *Sent* appears only after
  Gmail accepts the message, and a dropped connection mid-send is reconciled before any retry so nothing is
  sent twice. Survives a crash or a restart.
- **Two themes, done well.** Light and Dark, with *Follow system*. Bundled Inter and Source Serif 4 so it
  looks the same on every machine. Keyboard shortcuts you already know from Outlook.
- **Settings that do things.** Accounts and signatures, notifications, sync interval, mark-as-read timing,
  remote images, download size, working hours, reminders, and more. Every control takes effect immediately.

<p align="center">
  <img src="docs/screenshots/calendar-week-light.png" alt="Overlook calendar week view" width="920">
</p>
<p align="center"><sub>Week view with overlapping meetings, an all-day row, working hours, and today's marker.</sub></p>

<table>
  <tr>
    <td><img src="docs/screenshots/inbox-dark.png" alt="Inbox in Dark"></td>
    <td><img src="docs/screenshots/calendar-week-dark.png" alt="Calendar in Dark"></td>
  </tr>
  <tr>
    <td align="center"><sub>Dark theme</sub></td>
    <td align="center"><sub>Calendar in Dark</sub></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/compose-light.png" alt="Composer with recipient suggestions"></td>
    <td><img src="docs/screenshots/settings-light.png" alt="Settings dialog"></td>
  </tr>
  <tr>
    <td align="center"><sub>Composer with recipient suggestions</sub></td>
    <td align="center"><sub>Settings</sub></td>
  </tr>
</table>

## Install

**Arch Linux and Omarchy.** One command:

```bash
curl -fsSL https://raw.githubusercontent.com/last-refuge/overlook/main/install.sh | bash
```

It downloads the [latest release](https://github.com/last-refuge/overlook/releases/latest), verifies its
SHA-256 checksum, and installs the package with `pacman`, so `pacman` can update and remove it. It asks for
your password only for `pacman`. On Omarchy, press <kbd>Super</kbd>+<kbd>Space</kbd> and type "Overlook".
If no prebuilt package exists for your machine it builds one from source instead, which takes a few minutes.

Prefer to read it first? [`install.sh`](install.sh) is short. Options: `--dry-run` shows the plan and changes
nothing, `--prefix DIR` installs into a directory without administrator access, `--ref REF` builds a branch
or tag, `--uninstall` removes it, `--yes` skips confirmations.

**Any other Linux.** The same command builds from source into `~/.local` with no administrator access. You
need Rust (via [rustup](https://rustup.rs)), git, and development packages for Wayland, libxkbcommon,
fontconfig, FreeType, and a Vulkan driver. Or by hand:

```bash
make install PREFIX=~/.local
```

`--source` forces a build from source even when a release exists.

**Remove it:** `sudo pacman -Rns overlook` on Arch, or `./install.sh --uninstall`. Your mail and settings in
`~/.local/share/overlook` stay until you delete them.

### Updates

Overlook checks GitHub for a new release once a day (switch it off in Settings → About) and shows a banner
when there is one. **Update now** downloads the release, verifies its checksum, and installs it in place:
through a polkit password dialog when Overlook was installed with pacman, or by swapping the binary when it
lives in your own prefix. Then **Restart Overlook**. Nothing about you is sent during the check.

Overlook starts with a **sample mailbox** so you can see how it works before connecting anything. The sample
account's connection is a switch in the status bar; flip it to watch offline changes sync.

## Connect Gmail

Overlook talks to Gmail and Google Calendar directly through Google's APIs, from your machine, with an OAuth
client that belongs to you. Nothing passes through a Last Refuge server.

1. In [Google Cloud](https://console.cloud.google.com), create a project, enable the **Gmail API** and the
   **Google Calendar API**.
2. Configure the consent screen (External, Testing), add yourself as a test user, and add these scopes:
   `gmail.modify`, `gmail.send`, `calendar.events`, `calendar.readonly`, `openid`, `userinfo.email`.
3. Create an OAuth client of type **Desktop app** and download its JSON.
4. Save it as `~/.config/overlook/google-oauth-client.json`.
5. In Overlook, open the account menu at the bottom of the sidebar → **Add Gmail account…** and sign in in
   your browser. Repeat for more accounts.

Tokens are stored in your system keyring (GNOME Keyring or KWallet through the Secret Service), never in
files. In Google's *Testing* mode you will be asked to sign in again every seven days; publishing the app
in Google Cloud lifts that. The [native app guide](llm-docs/overlook-native-app.md#gmail) has the details.

## Keyboard

| Key | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>N</kbd> | New message |
| <kbd>Ctrl</kbd>+<kbd>R</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> / <kbd>Ctrl</kbd>+<kbd>F</kbd> | Reply / Reply all / Forward |
| <kbd>Ctrl</kbd>+<kbd>Enter</kbd> | Send |
| <kbd>Ctrl</kbd>+<kbd>B</kbd> / <kbd>I</kbd> / <kbd>K</kbd> | Bold / italic / link while writing |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>8</kbd> / <kbd>7</kbd> / <kbd>9</kbd> | Bulleted list / numbered list / quote |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> | Preview the message |
| <kbd>Ctrl</kbd>+<kbd>E</kbd> | Search |
| <kbd>J</kbd> / <kbd>K</kbd> | Next / previous conversation |
| <kbd>E</kbd> | Archive, or Move to Inbox when already archived |
| <kbd>Delete</kbd> | Move to Trash |
| <kbd>S</kbd> / <kbd>U</kbd> | Star / toggle read |
| <kbd>Ctrl</kbd>+<kbd>1</kbd> / <kbd>Ctrl</kbd>+<kbd>2</kbd> | Mail / Calendar |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>A</kbd> | New event |
| <kbd>←</kbd> <kbd>→</kbd> <kbd>T</kbd> <kbd>D</kbd> <kbd>W</kbd> <kbd>M</kbd> <kbd>A</kbd> | Calendar: move, today, day, week, month, agenda |
| <kbd>Ctrl</kbd>+<kbd>,</kbd> | Settings |
| <kbd>Esc</kbd> | Close composer or settings, clear search |

Drag a conversation onto Inbox, Archive, Trash, Starred, or a label in the sidebar to move it. Every move
has Undo.

## Built with

Rust and [GPUI](https://gpui.rs), Zed's GPU-accelerated UI framework, through [GPUI Kit](https://gpui-kit.com).
SQLite with FTS5 for local storage and search. No web view, no Electron.

```
crates/overlook-core       domain model, Focused Inbox rules, HTML sanitizer, time zones, preferences
crates/overlook-store      SQLite: messages, drafts, Outbox, pending operations, calendars, events, settings
crates/overlook-provider   provider traits, the Gmail and Google Calendar providers, the sample provider
crates/overlook-app        the GPUI application: shell, sync engine, keyring, sign-in, notifications
branding/                  brand system, fonts, logos, design previews
packaging/                 desktop entry, AppStream metadata, PKGBUILD, Flatpak manifest draft
llm-docs/                  product plan, brand system, architecture and implementation notes
```

Run the tests with `cargo test --workspace`. Received HTML is rendered natively after sanitization; complex
newsletter layouts are flattened rather than reproduced pixel for pixel, a deliberate trade for having no
browser engine in a mail client.

## Status

Early daily use by its author on Omarchy (Hyprland, Wayland). Gmail accounts work end to end; IMAP/SMTP and
POP3 are planned next, as are attachment previews and a Flatpak build. GNOME, KDE,
and screen-reader testing are still to be done. Open an issue with what you did, what you saw, and what you
expected.

## Brand and credits

The horizon mark, wordmark, palette, and typography are documented in [`llm-docs/overlook-brand.md`](llm-docs/overlook-brand.md).
Fonts: [Inter](https://rsms.me/inter/) and [Source Serif 4](https://github.com/adobe-fonts/source-serif) under the
SIL Open Font License. Icons: [Lucide](https://lucide.dev) (ISC). Overlook is a Last Refuge product.

## License

Copyright © 2026 Last Refuge. All rights reserved. Source is published for transparency and review; a
license for redistribution has not been chosen yet.
