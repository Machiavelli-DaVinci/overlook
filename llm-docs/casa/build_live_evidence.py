#!/usr/bin/env python3
"""Render supplemental evidence from observed live-session records, never credentials."""
import hashlib
import json
from pathlib import Path
import subprocess
import textwrap
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent

def font(name, size):
    return ImageFont.truetype(subprocess.check_output(['fc-match', '-f', '%{file}', name], text=True), size)

FONTS = {'title': font('sans:bold', 39), 'heading': font('sans:bold', 26),
         'body': font('sans', 24), 'small': font('sans', 20), 'code': font('monospace', 19)}

def render(control, sections):
    im = Image.new('RGB', (1600, 12000), 'white')
    d = ImageDraw.Draw(im)
    y = 52
    def text(value, style='body'):
        nonlocal y
        f = FONTS[style]
        for para in str(value).splitlines() or ['']:
            if style == 'code':
                lines = textwrap.wrap(para, int(1472 / d.textlength('M', font=f)),
                                      replace_whitespace=False, drop_whitespace=False) or ['']
            else:
                lines, current = [], ''
                for word in para.split():
                    nxt = (current + ' ' + word).strip()
                    if current and d.textlength(nxt, font=f) > 1472:
                        lines.append(current)
                        current = word
                    else:
                        current = nxt
                lines.append(current)
            for line in lines:
                d.text((64, y), line, font=f, fill='#172b35')
                y += int(f.size * 1.45)
    text('OVERLOOK | CASA ' + control + ' | LIVE VERIFICATION', 'title')
    text('22 October 2026 • supplemental evidence document • timestamps are UTC', 'small')
    text('Actual user-completed Google login; private isolated application profile.', 'small')
    text('This is a rendered evidence document, not an application screenshot.', 'small')
    for title, body, style in sections:
        y += 25
        text(title, 'heading')
        y += 8
        text(body, style)
    y += 25
    text('No secret values were retrieved from the OS keyring. Account identifiers are redacted.', 'small')
    text('This supplement updates the earlier synthetic/source evidence. TAC determines acceptance.', 'small')
    assert y < 11950
    out = HERE / 'evidence' / ('casa-' + control + '-live.png')
    im.crop((0, 0, 1600, y + 45)).save(out, optimize=True)
    return out

login = json.loads((HERE / 'live-login-verification.json').read_text())
assert login['gmail_account_count'] == 1
assert login['keyring_metadata']['unlocked_items'] == 1
assert not any(login['credential_pattern_matches'].values())
logpath = HERE / 'logs/live-google-login.txt'
log = logpath.read_text()
assert hashlib.sha256(logpath.read_bytes()).hexdigest() == login['log_redacted_sha256']
sample = [line for line in log.splitlines() if 'first frame' in line or 'queued for download' in line or 'synced 0 contacts' in line]
provenance = ('Overlook 1.5.5; source commit ' + login['source_commit'] +
              '; patched Cargo.lock in the working tree. Executable SHA-256: ' + login['binary_sha256'] +
              '. This build was launched locally for testing; publication is not asserted.')
login_sections = [
    ('Live observation', f"At {login['observed_utc']}, the user reported successful Google sign-in. "
     f"Read-only inspection of the isolated profile found one Gmail account, {login['downloaded_message_count']} "
     f"downloaded messages, sync state {login['sync_state']['state']}, and a successful sync timestamp. "
     'Secret Service SearchItems returned one unlocked OAuth credential item for that account. '
     'The secret value was not read.', 'body'),
    ('Build identity', provenance, 'body'),
    ('Actual login-session log excerpt', '\n'.join(sample), 'code'),
    ('Credential logging checks', 'No matches were found for Google access-token and refresh-token patterns, '
     'Authorization: Bearer headers, or access_token/refresh_token/client_secret/code_verifier assignments in '
     'the raw application log. The saved sample redacts account identifiers and names. Existing source and '
     'synthetic trace-level regression evidence documents redacted Debug output and protocol/keyring log filtering.', 'body'),
    ('Limits', 'The live run used info-level logging and has no explicit OAuth-success log event; authenticated '
     'sync and keyring metadata corroborate sign-in. Pattern checks alone cannot prove that every possible secret '
     'format is absent. This application has no payment process. No Google password or credential values are '
     'included in the evidence.', 'body'),
    ('Retained evidence', 'logs/live-google-login.txt\nSHA-256: ' + login['log_redacted_sha256'] +
     '\nlive-login-verification.json contains the observed counts and checks.', 'small'),
]
out = render('6.5.1', login_sections)
responses = [{
    'id': '6.5.1', 'applicable': '1',
    'comment': 'Overlook native Linux desktop app. OAuth tokens, client-secret values and PKCE verifiers have '
    'redacted Debug output; both log sinks exclude protocol/keyring logging even when trace is requested. '
    'A live Google sign-in was completed by the owner on 2026-10-22 using an isolated profile and the patched '
    'v1.5.5 build. Authenticated sync downloaded 93 messages and reported up-to-date; OS-keyring metadata '
    'confirmed one stored OAuth item without reading its secret. A real login-session log sample is attached '
    'with account identifiers redacted. Credential-pattern checks found no matches; this is supplementary '
    'evidence, not proof against every possible secret format. Live logging was info level; the earlier '
    'synthetic regression tests trace-level filtering. No payment process exists. See the live supplement '
    'for build identity and limitations.',
    'validation': 'User-completed live Google login, authenticated sync, OS-keyring metadata, and sanitized '
    'real log verified on 2026-10-22. Secret values were not retrieved. Earlier synthetic trace-level test remains supplemental.',
    'evidence': str(out),
}]

removal_path = HERE / 'live-removal-verification.json'
if removal_path.exists():
    removal = json.loads(removal_path.read_text())
    assert removal['gmail_account_count'] == 0
    assert removal['account_keyring_items'] == 0
    assert removal['account_message_count'] == 0
    sections = [
        ('Before removal', 'The owner completed Google sign-in in the isolated profile. Authenticated sync '
         'downloaded 93 messages and became up-to-date. One unlocked OAuth credential item existed in the '
         'OS keyring; no secret value was retrieved.', 'body'),
        ('After removal', f"At {removal['observed_utc']}, after the owner used Remove account, read-only checks "
         'found zero Gmail accounts, zero cached messages for the removed account, zero sync-state rows for '
         'that account, and zero matching Secret Service items across all credential kinds. The local account '
         'and keyring cleanup completed in the real application.', 'body'),
        ('Revocation path and scope', 'The reviewed remove-account path calls the shared Gmail client disconnect '
         'before deleting credentials or account data. That disables further provider use, drains in-flight '
         'work and posts to Google’s revoke endpoint. The code permits cleanup only after HTTP 200, or '
         'HTTP 400 with invalid_token (already invalid). The completed cleanup therefore demonstrates that '
         'this guarded revocation path returned success. No raw HTTP trace or independent replay of a revoked '
         'token was captured; no secret values were extracted for replay.', 'body'),
        ('Build identity', provenance, 'body'),
        ('Retained evidence', 'live-login-verification.json\nlive-removal-verification.json\n'
         'Source: gmail/api.rs disconnect_with, gmail/oauth.rs revoke, and ui/mod.rs remove_account.\n'
         'Earlier evidence contains source excerpts and failure/retry/cloned-client regression results.', 'small'),
    ]
    out = render('2.2.1', sections)
    responses.append({
        'id': '2.2.1', 'applicable': '1',
        'comment': 'Overlook native Linux desktop app. Remove account disables the shared Gmail client, drains '
        'in-flight work, revokes the Google grant over HTTPS, clears in-memory tokens, then deletes OS-keyring '
        'entries and local account data. Failed revocation or keyring deletion reports incomplete removal '
        'and retains retry information. On 2026-10-22 the owner completed a live sign-in/removal test: '
        'authenticated sync downloaded 93 messages; before removal one OAuth keyring item existed; afterward '
        'the Gmail account, its cached messages/sync state and all matching keyring items were absent. '
        'The completed guarded cleanup demonstrates the revocation path returned success; no independent '
        'revoked-token replay or HTTP trace was captured. Removal requires connectivity and affects the '
        'user’s grant across this Google project’s devices/scopes. Provider passwords are removed locally, '
        'not changed at their issuer. See live supplement and source/regression evidence.',
        'validation': 'Live user-driven sign-in/removal and before/after profile/keyring metadata verified. '
        'Remote revocation success is inferred from the guarded application path; no independent token replay performed.',
        'evidence': str(out),
    })

assert all(len(x['comment']) <= 1500 for x in responses)
(HERE / 'live-responses.json').write_text(json.dumps(responses, indent=2, ensure_ascii=False) + '\n')
canonical_path = HERE / 'responses.json'
if canonical_path.exists():
    replacements = {x['id']: x for x in responses}
    canonical = json.loads(canonical_path.read_text())
    canonical_path.write_text(json.dumps([replacements.get(x['id'], x) for x in canonical],
                                         indent=2, ensure_ascii=False) + '\n')
matrix_path = HERE / 'control-matrix.md'
if matrix_path.exists():
    matrix = matrix_path.read_text()
    for item in responses:
        start = matrix.index('## ' + item['id'] + ' ')
        end = matrix.find('\n## ', start + 1)
        if end < 0:
            end = len(matrix)
        title = matrix[start:].splitlines()[0]
        section = (title + '\n\nApplicable: Yes\n\n' + item['comment'] + '\n\n' +
                   item['validation'] + '\n\n[Initial source/test evidence](evidence/casa-' + item['id'] +
                   '.png) · [Live verification supplement](evidence/casa-' + item['id'] + '-live.png)\n')
        matrix = matrix[:start] + section + matrix[end:]
    matrix_path.write_text(matrix)
print('Rendered live supplements for: ' + ', '.join(x['id'] for x in responses))
