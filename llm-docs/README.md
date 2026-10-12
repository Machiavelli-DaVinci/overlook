# Overlook Documentation

Overlook is a native Linux mail client in Rust, with an Outlook-inspired workflow and an alpine identity in the Last Refuge family.

- [Session handoff](session-handoff.md): comprehensive context and the recommended continuation milestone for another session.
- [Foundation implementation plan](foundation-implementation-plan.md): validated technical foundation (Rust + GPUI), tradeoffs, workspace rules, milestone sequence, and status log.
- [Native app guide](overlook-native-app.md): building, running, testing, architecture, keyboard shortcuts, and known gaps of the GPUI application in `crates/`.
- [Product and implementation plan](native-linux-mail-client-plan.md): phased provider support, architecture, offline synchronization, Light/Dark theming, and release criteria.
- [Brand system](overlook-brand.md): the selected identity, positioning, logo usage, colors, typography, voice, and product design guidance.
- [Brand asset guide](overlook-brand-assets.md): the asset inventory, implementation handoff, regeneration instructions, and archive packaging.
- [Art direction](overlook-art-direction.md): original landscape artwork provenance and exact generation prompt.

Application code lives in [`crates/`](../crates/), packaging drafts in [`packaging/`](../packaging/), and helper scripts in [`scripts/`](../scripts/). Reusable assets and source files are in [`branding/`](../branding/). Start with the [visual brand book](../branding/index.html), [mail design preview](../branding/mail-preview.html), or [portable brand kit](../branding/overlook-brand-kit.zip).
