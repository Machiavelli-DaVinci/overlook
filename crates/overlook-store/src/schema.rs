//! Schema migrations. `PRAGMA user_version` tracks the applied version.

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[V1];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if version > current {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", version)?;
        }
    }
    Ok(())
}

const V1: &str = r#"
BEGIN;
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    display_name TEXT NOT NULL,
    email TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE labels (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    provider_id TEXT,
    color TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX labels_account ON labels(account_id, kind);

-- Downloaded mail. Replaceable cache, except that rows referenced by pending
-- operations must not be evicted (enforced by the eviction routine, not the schema).
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    provider_id TEXT,
    from_json TEXT NOT NULL,
    to_json TEXT NOT NULL,
    cc_json TEXT NOT NULL,
    bcc_json TEXT NOT NULL,
    reply_to_json TEXT,
    subject TEXT NOT NULL,
    snippet TEXT NOT NULL,
    body_text TEXT,
    body_html TEXT,
    search_text TEXT NOT NULL,
    date INTEGER NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_starred INTEGER NOT NULL DEFAULT 0,
    is_outgoing INTEGER NOT NULL DEFAULT 0,
    rfc_message_id TEXT,
    in_reply_to TEXT
);
CREATE INDEX messages_conversation ON messages(account_id, conversation_id);
CREATE INDEX messages_date ON messages(account_id, date DESC);
CREATE UNIQUE INDEX messages_provider ON messages(account_id, provider_id) WHERE provider_id IS NOT NULL;

CREATE TABLE message_labels (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
);
CREATE INDEX message_labels_label ON message_labels(label_id);

CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    local_path TEXT,
    provider_id TEXT
);

-- Protected local work. Never evicted as cache.
CREATE TABLE drafts (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    conversation_id TEXT,
    in_reply_to TEXT,
    to_json TEXT NOT NULL,
    cc_json TEXT NOT NULL,
    bcc_json TEXT NOT NULL,
    subject TEXT NOT NULL,
    body_text TEXT NOT NULL,
    body_html TEXT,
    state_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    provider_id TEXT
);
CREATE INDEX drafts_account ON drafts(account_id, updated_at DESC);

CREATE TABLE draft_attachments (
    id TEXT PRIMARY KEY,
    draft_id TEXT NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    local_path TEXT NOT NULL
);

CREATE TABLE pending_operations (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind_json TEXT NOT NULL,
    state TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (account_id, seq)
);

CREATE TABLE sync_state (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    state_json TEXT NOT NULL,
    last_successful_sync INTEGER,
    checkpoint TEXT
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Full-text index over subject, people, and body text (external content table).
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, search_text,
    content='messages', content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, search_text) VALUES (new.rowid, new.subject, new.search_text);
END;
CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, search_text) VALUES ('delete', old.rowid, old.subject, old.search_text);
END;
CREATE TRIGGER messages_fts_update AFTER UPDATE OF subject, search_text ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, search_text) VALUES ('delete', old.rowid, old.subject, old.search_text);
    INSERT INTO messages_fts(rowid, subject, search_text) VALUES (new.rowid, new.subject, new.search_text);
END;
COMMIT;
"#;
