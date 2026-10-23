//! Durable CalDAV configuration. Credentials are referenced, never stored here.

use super::*;

pub(super) fn read_connection(
    conn: &Connection,
    account: &AccountId,
) -> Result<Option<CalendarConnection>> {
    let row = conn
        .query_row("SELECT id, settings_json, enabled, generation FROM calendar_connections WHERE account_id=?1", [&account.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, r.get::<_, u32>(3)?))
        })
        .optional()?;
    row.map(|(id, json, enabled, generation)| {
        Ok(CalendarConnection {
            id: CalendarConnectionId(id),
            account_id: account.clone(),
            settings: serde_json::from_str(&json)?,
            enabled,
            generation,
        })
    })
    .transpose()
}

impl Store {
    pub fn calendar_connection(&self, account: &AccountId) -> Result<Option<CalendarConnection>> {
        self.with_read(|conn| read_connection(conn, account))
    }

    /// Saves setup atomically, rejecting a dialog or background result based on old settings.
    /// `None` creates a connection; updates require the generation last read by the caller.
    /// This does not connect, store a secret, or modify the account's mail settings.
    pub fn save_calendar_connection(
        &self,
        account: &AccountId,
        settings: &CalDavSettings,
        enabled: bool,
        expected_generation: Option<u32>,
    ) -> Result<CalendarConnection> {
        settings
            .validate()
            .map_err(|e| StoreError::Invalid(e.into()))?;
        self.with_tx(|tx| {
            let existing = read_connection(tx, account)?;
            if existing.as_ref().map(|c| c.generation) != expected_generation {
                return Err(StoreError::Invalid("Calendar settings changed. Reload them before saving.".into()));
            }
            let generation =
                expected_generation.unwrap_or(0).checked_add(1).ok_or_else(|| StoreError::Invalid("Calendar connection generation exhausted.".into()))?;
            let id = existing.map(|c| c.id).unwrap_or_else(CalendarConnectionId::generate);
            tx.execute(
                "INSERT INTO calendar_connections(id, account_id, settings_json, enabled, generation) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(account_id) DO UPDATE SET settings_json=excluded.settings_json, enabled=excluded.enabled, generation=excluded.generation",
                params![id.0, account.0, serde_json::to_string(settings)?, enabled, generation],
            )?;
            Ok(CalendarConnection { id, account_id: account.clone(), settings: settings.clone(), enabled, generation })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: AccountId::from("imap:test@example.com"),
            provider: ProviderKind::Imap,
            display_name: "Test".into(),
            email: "test@example.com".into(),
            created_at: 1,
        }
    }

    fn settings() -> CalDavSettings {
        CalDavSettings {
            server_url: "https://caldav.fastmail.com/".into(),
            username: "test@example.com".into(),
            credential_source: CalendarCredentialSource::Separate,
        }
    }

    #[test]
    fn setup_survives_reopen_and_stale_configuration_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "overlook-caldav-{}.sqlite",
            CalendarConnectionId::generate()
        ));
        let acct = account();
        let first = {
            let store = Store::open(&path).unwrap();
            store.upsert_account(&acct).unwrap();
            store.set_setting("mail-sentinel", &"unchanged").unwrap();
            store
                .save_calendar_connection(&acct.id, &settings(), true, None)
                .unwrap()
        };
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(
                store.calendar_connection(&acct.id).unwrap(),
                Some(first.clone())
            );
            let mut changed = settings();
            changed.username = "different@example.com".into();
            changed.credential_source = CalendarCredentialSource::IncomingMail;
            let next = store
                .save_calendar_connection(&acct.id, &changed, false, Some(first.generation))
                .unwrap();
            assert_eq!(next.id, first.id);
            assert_eq!(next.generation, 2);
            assert!(!next.enabled);
            assert_eq!(next.secret_kind(), first.secret_kind());
            assert!(store
                .save_calendar_connection(&acct.id, &settings(), true, Some(first.generation))
                .is_err());
            assert!(store
                .save_calendar_connection(&acct.id, &settings(), true, None)
                .is_err());
            assert_eq!(store.calendar_connection(&acct.id).unwrap(), Some(next));
            assert_eq!(
                store.setting::<String>("mail-sentinel").unwrap().as_deref(),
                Some("unchanged")
            );
            store.delete_account(&acct.id).unwrap();
            assert!(store.calendar_connection(&acct.id).unwrap().is_none());
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_or_orphan_configuration_is_not_saved() {
        let store = Store::open_in_memory().unwrap();
        let acct = account();
        assert!(store
            .save_calendar_connection(&acct.id, &settings(), true, None)
            .is_err());
        store.upsert_account(&acct).unwrap();
        let mut invalid = settings();
        invalid.server_url = "https://user:secret@example.com/".into();
        assert!(store
            .save_calendar_connection(&acct.id, &invalid, true, None)
            .is_err());
        assert!(store.calendar_connection(&acct.id).unwrap().is_none());
    }
}
