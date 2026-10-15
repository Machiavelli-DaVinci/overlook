#!/usr/bin/env bash
# Runs the ignored real-server tests for one account and prints a row for the
# interoperability matrix in llm-docs/imap-pop3-implementation-plan.md.
#
#   OVERLOOK_IMAP_TEST_HOST=imap.example.com OVERLOOK_IMAP_TEST_USER=you@example.com \
#   OVERLOOK_IMAP_TEST_PASSWORD=… [OVERLOOK_IMAP_TEST_SECURITY=tls|starttls] \
#   [OVERLOOK_SMTP_TEST_HOST=… OVERLOOK_SMTP_TEST_USER=… OVERLOOK_SMTP_TEST_PASSWORD=… OVERLOOK_SMTP_TEST_TO=…] \
#   [OVERLOOK_POP3_TEST_HOST=… OVERLOOK_POP3_TEST_USER=… OVERLOOK_POP3_TEST_PASSWORD=…] \
#   scripts/interop-matrix.sh "Fastmail"
#
# Each protocol runs only when its *_HOST is set. Passwords are read from the environment
# and never written anywhere; test output goes to /tmp/overlook-interop-<name>.log.
set -u
NAME="${1:-server}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG="/tmp/overlook-interop-$(echo "$NAME" | tr -c 'A-Za-z0-9' '_').log"
: > "$LOG"
row=()
run() { # label, crate, filter
  local label="$1" crate="$2" filter="$3"
  echo "== $label ($crate $filter)" >> "$LOG"
  if (cd "$ROOT" && cargo test -p "$crate" "$filter" -- --ignored --nocapture >> "$LOG" 2>&1); then
    row+=("$label: pass")
  else
    row+=("$label: FAIL (see $LOG)")
  fi
}
if [ -n "${OVERLOOK_IMAP_TEST_HOST:-}" ]; then
  run "IMAP read (${OVERLOOK_IMAP_TEST_SECURITY:-tls})" overlook-provider real_imap_server_round_trip
  run "IMAP write" overlook-provider real_imap_server_write_round_trip
  run "IMAP sync through the store" overlook imap_account_syncs_through_the_store_across_a_restart
fi
if [ -n "${OVERLOOK_SMTP_TEST_HOST:-}" ]; then
  run "SMTP (${OVERLOOK_SMTP_TEST_SECURITY:-starttls})" overlook-provider real_smtp_server_round_trip
fi
if [ -n "${OVERLOOK_POP3_TEST_HOST:-}" ]; then
  run "POP3 (${OVERLOOK_POP3_TEST_SECURITY:-tls})" overlook-provider real_pop3_server_round_trip
fi
echo "| $NAME | $(date -u +%Y-%m-%d) | $(IFS='; '; echo "${row[*]}") |"
