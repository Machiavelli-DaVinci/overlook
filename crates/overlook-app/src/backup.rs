//! "Back up mail": a copy of the profile that opens on a clean data directory. For POP3
//! accounts the profile is the only copy of their mail, so this is the user's safety net
//! until a restore UI exists (restore: stop Overlook, replace the data directory).

use overlook_store::Store;
use std::path::Path;

/// Copies a directory tree, replacing files that already exist at the destination.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<usize> {
    if !from.is_dir() {
        return Ok(0);
    }
    std::fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Writes a backup of the profile into `target`: `overlook.sqlite` (a consistent copy made
/// with `VACUUM INTO`), `raw/` (POP3 messages) and `attachments/`. The result reads back
/// with `Store::open` on a clean data directory. Returns the number of files written.
pub fn write_backup(store: &Store, data_dir: &Path, target: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(target).map_err(|e| format!("could not create {}: {e}", target.display()))?;
    store.backup_to(&target.join("overlook.sqlite")).map_err(|e| format!("could not copy the database: {e}"))?;
    let mut files = 1;
    for sub in ["raw", "attachments"] {
        files += copy_tree(&data_dir.join(sub), &target.join(sub)).map_err(|e| format!("could not copy {sub}: {e}"))?;
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use overlook_core::*;

    #[test]
    fn backup_copies_database_raw_and_attachments_and_reopens() {
        let base = std::env::temp_dir().join(format!("overlook-backup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let data_dir = base.join("data");
        std::fs::create_dir_all(data_dir.join("raw/pop3_j")).unwrap();
        std::fs::create_dir_all(data_dir.join("attachments/a/b")).unwrap();
        std::fs::write(data_dir.join("raw/pop3_j/one.eml"), b"Subject: one\r\n\r\nbody").unwrap();
        std::fs::write(data_dir.join("attachments/a/b/file.pdf"), b"%PDF").unwrap();
        let store = Store::open(&data_dir.join("overlook.sqlite")).unwrap();
        let account = Account { id: AccountId::from("pop3:j"), provider: ProviderKind::Pop3, display_name: "J".into(), email: "j@example.com".into(), created_at: 0 };
        store.upsert_account(&account).unwrap();

        let target = base.join("backup");
        assert_eq!(write_backup(&store, &data_dir, &target).unwrap(), 3);
        assert_eq!(std::fs::read(target.join("raw/pop3_j/one.eml")).unwrap(), b"Subject: one\r\n\r\nbody");
        assert_eq!(std::fs::read(target.join("attachments/a/b/file.pdf")).unwrap(), b"%PDF");
        // The copy opens as a profile of its own.
        let restored = Store::open(&target.join("overlook.sqlite")).unwrap();
        assert_eq!(restored.accounts().unwrap()[0].id, account.id);
        drop(restored);
        // Running it again over the same folder replaces the files rather than failing.
        assert_eq!(write_backup(&store, &data_dir, &target).unwrap(), 3);
        // A profile without raw/ or attachments/ still backs up.
        let bare = base.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        let bare_store = Store::open(&bare.join("overlook.sqlite")).unwrap();
        assert_eq!(write_backup(&bare_store, &bare, &base.join("bare-backup")).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
