PREFIX ?= /usr/local
CARGO ?= cargo

.PHONY: build release test install uninstall run screenshots

build:
	$(CARGO) build -p overlook

release:
	$(CARGO) build --release --locked -p overlook

test:
	$(CARGO) test --workspace

run:
	$(CARGO) run -p overlook

install: release
	install -Dm755 target/release/overlook $(DESTDIR)$(PREFIX)/bin/overlook
	install -Dm644 packaging/ai.lastrefuge.Overlook.desktop $(DESTDIR)$(PREFIX)/share/applications/ai.lastrefuge.Overlook.desktop
	install -Dm644 packaging/ai.lastrefuge.Overlook.metainfo.xml $(DESTDIR)$(PREFIX)/share/metainfo/ai.lastrefuge.Overlook.metainfo.xml
	install -Dm644 branding/assets/overlook-app-icon.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/ai.lastrefuge.Overlook.svg
	for s in 16 24 32 48 64 128 256 512; do \
		install -Dm644 branding/assets/overlook-icon-$$s.png $(DESTDIR)$(PREFIX)/share/icons/hicolor/$${s}x$${s}/apps/ai.lastrefuge.Overlook.png; \
	done
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null
	-gtk-update-icon-cache -q $(DESTDIR)$(PREFIX)/share/icons/hicolor 2>/dev/null

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/overlook \
		$(DESTDIR)$(PREFIX)/share/applications/ai.lastrefuge.Overlook.desktop \
		$(DESTDIR)$(PREFIX)/share/metainfo/ai.lastrefuge.Overlook.metainfo.xml \
		$(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/ai.lastrefuge.Overlook.svg \
		$(DESTDIR)$(PREFIX)/share/icons/hicolor/*/apps/ai.lastrefuge.Overlook.png

screenshots:
	@echo "scripts/capture-window.sh <profile-dir> <out.png> [keys]  (Hyprland; see llm-docs/overlook-native-app.md)"
