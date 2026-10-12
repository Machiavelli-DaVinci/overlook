//! Synchronization engine for one account.
//!
//! Replays pending operations in sequence order against the provider, records provider
//! acceptance before anything is called Sent, and keeps uncertain sends distinct from
//! failures. Runs on a background thread; the window reloads from the store afterwards.

use overlook_core::time::now;
use overlook_core::*;
use overlook_provider::{MailProvider, ProviderError, SendOutcome};
use overlook_store::Store;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Default, Clone)]
pub struct SyncOutcome {
    pub state: Option<SyncState>,
    pub applied: usize,
    pub sent: Vec<String>,
    pub uncertain: Vec<String>,
    pub failed: Vec<String>,
    pub stopped_offline: bool,
}

pub async fn run_sync<P: MailProvider>(store: Arc<Store>, provider: P, account: Account) -> SyncOutcome {
    let mut outcome = SyncOutcome::default();
    let acct = account.id.clone();

    if let Err(err) = provider.probe().await {
        let state = match err {
            ProviderError::SignInRequired => SyncState::SignInRequired,
            _ => SyncState::Offline,
        };
        let _ = store.set_sync_state(&acct, &state, None);
        outcome.state = Some(state);
        outcome.stopped_offline = true;
        return outcome;
    }
    let _ = store.set_sync_state(&acct, &SyncState::Syncing, None);

    let sent_label = store.label_by_kind(&acct, LabelKind::Sent).ok().flatten().map(|l| l.id);
    let ops = match store.pending_operations(&acct) {
        Ok(ops) => ops,
        Err(e) => {
            let state = SyncState::SyncFailed(e.to_string());
            let _ = store.set_sync_state(&acct, &state, None);
            outcome.state = Some(state);
            return outcome;
        }
    };

    let mut final_state = SyncState::UpToDate;
    for op in ops {
        if op.state == OperationState::Applied {
            continue;
        }
        let _ = store.set_operation_state(&op.id, OperationState::InFlight, None);
        match &op.kind {
            OperationKind::SendDraft { draft_id } => {
                let Some(draft) = store.draft(draft_id).ok().flatten() else {
                    // The draft was discarded; nothing to send.
                    let _ = store.set_operation_state(&op.id, OperationState::Applied, None);
                    continue;
                };
                let subject = draft.display_subject();
                let _ = store.set_draft_state(&draft.id, &DraftState::Sending);
                // An operation that was interrupted earlier must be reconciled first.
                let needs_reconcile = op.state == OperationState::Uncertain || draft.state == DraftState::Uncertain;
                let send_result = if needs_reconcile {
                    match provider.reconcile_send(&account, &draft).await {
                        Ok(SendOutcome::Delivered(provider_id)) => {
                            let rfc = format!("<{}@overlook.sample>", draft.id);
                            Ok(overlook_provider::SentReceipt {
                                message: overlook_provider::sample::sent_message(&account, &draft, &provider_id, &rfc, sent_label.as_ref(), now()),
                                provider_id,
                                rfc_message_id: Some(rfc),
                            })
                        }
                        Ok(SendOutcome::NotDelivered) => provider.send(&account, &draft, sent_label.as_ref()).await,
                        Err(e) => Err(e),
                    }
                } else {
                    provider.send(&account, &draft, sent_label.as_ref()).await
                };
                match send_result {
                    Ok(receipt) => {
                        if let Err(e) = store.complete_send(&draft.id, &op.id, &receipt.message) {
                            warn!("complete_send failed: {e}");
                            let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&e.to_string()));
                            let _ = store.set_draft_state(&draft.id, &DraftState::Failed(e.to_string()));
                            outcome.failed.push(subject);
                        } else {
                            info!("sent {subject}");
                            outcome.sent.push(subject);
                            outcome.applied += 1;
                        }
                    }
                    Err(ProviderError::Uncertain) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Uncertain, Some("connection dropped during sending"));
                        let _ = store.set_draft_state(&draft.id, &DraftState::Uncertain);
                        outcome.uncertain.push(subject);
                        // Do not continue replaying behind an uncertain send; reconcile next cycle.
                        final_state = SyncState::Offline;
                        break;
                    }
                    Err(ProviderError::Offline) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, None);
                        let _ = store.set_draft_state(&draft.id, &DraftState::Queued);
                        final_state = SyncState::Offline;
                        outcome.stopped_offline = true;
                        break;
                    }
                    Err(ProviderError::SignInRequired) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, None);
                        let _ = store.set_draft_state(&draft.id, &DraftState::Queued);
                        final_state = SyncState::SignInRequired;
                        break;
                    }
                    Err(ProviderError::Temporary(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some(&reason));
                        let _ = store.set_draft_state(&draft.id, &DraftState::Queued);
                        final_state = SyncState::SyncFailed(reason);
                        break;
                    }
                    Err(ProviderError::Rejected(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&reason));
                        let _ = store.set_draft_state(&draft.id, &DraftState::Failed(reason));
                        outcome.failed.push(subject);
                    }
                }
            }
            kind => {
                let messages: Vec<Message> = kind
                    .message_ids()
                    .iter()
                    .filter_map(|id| store.message(id).ok().flatten())
                    .collect();
                match provider.apply(&account, &op, &messages).await {
                    Ok(()) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Applied, None);
                        outcome.applied += 1;
                    }
                    Err(ProviderError::Offline) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, None);
                        final_state = SyncState::Offline;
                        outcome.stopped_offline = true;
                        break;
                    }
                    Err(ProviderError::SignInRequired) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, None);
                        final_state = SyncState::SignInRequired;
                        break;
                    }
                    Err(ProviderError::Temporary(reason)) | Err(ProviderError::Rejected(reason)) => {
                        let _ = store.set_operation_state(&op.id, OperationState::Failed, Some(&reason));
                        outcome.failed.push(kind.describe());
                        final_state = SyncState::SyncFailed(reason);
                        break;
                    }
                    Err(ProviderError::Uncertain) => {
                        // Idempotent operations can simply be retried.
                        let _ = store.set_operation_state(&op.id, OperationState::Pending, Some("connection dropped"));
                        final_state = SyncState::Offline;
                        break;
                    }
                }
            }
        }
    }

    let success_at = if final_state == SyncState::UpToDate { Some(now()) } else { None };
    let _ = store.set_sync_state(&acct, &final_state, success_at);
    let _ = store.prune_applied(now() - 7 * 86_400_000);
    outcome.state = Some(final_state);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use overlook_provider::sample::{sample_mailbox, SampleProvider};
    use std::time::Duration;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = Box::pin(f);
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    fn seeded() -> (Arc<Store>, Account, SampleProvider) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let account = Account {
            id: AccountId::from("sample"),
            provider: ProviderKind::Sample,
            display_name: "Jason Alexander".into(),
            email: "jason@example.com".into(),
            created_at: 0,
        };
        store.upsert_account(&account).unwrap();
        let snap = sample_mailbox(&account, now());
        for l in &snap.labels {
            store.upsert_label(l).unwrap();
        }
        store.upsert_messages(&snap.messages).unwrap();
        (store, account, SampleProvider::new().with_latency(Duration::ZERO))
    }

    fn queued_draft(store: &Store, account: &Account) -> Draft {
        let mut draft = Draft::new(account.id.clone(), DraftKind::New, now());
        draft.to = vec![Address::named("Maya Chen", "maya@northstar.example")];
        draft.subject = "Agenda".into();
        draft.body_text = "A few things to cover.".into();
        store.save_draft(&draft).unwrap();
        store.queue_send(&draft).unwrap();
        draft
    }

    #[test]
    fn offline_sync_leaves_everything_queued() {
        let (store, account, provider) = seeded();
        let first = store.messages_for_account(&account.id).unwrap()[0].id.clone();
        store.apply_change(&account.id, overlook_store::LocalChange::SetStarred { message_ids: vec![first], starred: true }).unwrap();
        queued_draft(&store, &account);
        let outcome = block_on(run_sync(store.clone(), provider, account.clone()));
        assert!(outcome.stopped_offline);
        assert_eq!(store.pending_count(&account.id).unwrap(), 2);
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::Offline);
        assert_eq!(store.drafts(&account.id, true).unwrap()[0].state, DraftState::Queued);
    }

    #[test]
    fn replay_is_ordered_and_send_needs_provider_acceptance() {
        let (store, account, provider) = seeded();
        let first = store.messages_for_account(&account.id).unwrap()[0].id.clone();
        store.apply_change(&account.id, overlook_store::LocalChange::SetStarred { message_ids: vec![first.clone()], starred: true }).unwrap();
        let draft = queued_draft(&store, &account);
        store.apply_change(&account.id, overlook_store::LocalChange::Archive { message_ids: vec![first] }).unwrap();
        provider.set_online(true);
        let outcome = block_on(run_sync(store.clone(), provider.clone(), account.clone()));
        assert_eq!(outcome.applied, 3);
        assert_eq!(outcome.sent, vec!["Agenda".to_owned()]);
        assert_eq!(store.pending_count(&account.id).unwrap(), 0);
        assert!(store.draft(&draft.id).unwrap().is_none(), "draft row removed after acceptance");
        let sent = store.conversations(&account.id, &account.email, &overlook_store::MailboxView::Kind(LabelKind::Sent), overlook_store::ListFilter::All).unwrap();
        assert!(sent.iter().any(|c| c.subject == "Agenda"));
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::UpToDate);
        let applied = provider.applied_operations();
        assert_eq!(applied.len(), 2, "two idempotent operations reached the provider in order");
    }

    #[test]
    fn dropped_connection_during_send_is_uncertain_then_reconciled_without_duplicates() {
        let (store, account, provider) = seeded();
        let draft = queued_draft(&store, &account);
        provider.set_online(true);
        provider.drop_next_send();
        let outcome = block_on(run_sync(store.clone(), provider.clone(), account.clone()));
        assert_eq!(outcome.uncertain, vec!["Agenda".to_owned()]);
        assert_eq!(store.draft(&draft.id).unwrap().unwrap().state, DraftState::Uncertain);
        let ops = store.pending_operations(&account.id).unwrap();
        assert_eq!(ops[0].state, OperationState::Uncertain);

        // Next cycle reconciles instead of blindly resending.
        let outcome = block_on(run_sync(store.clone(), provider.clone(), account.clone()));
        assert_eq!(outcome.sent, vec!["Agenda".to_owned()]);
        assert!(store.draft(&draft.id).unwrap().is_none());
        let sent: Vec<_> = store
            .messages_for_account(&account.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.is_outgoing && m.subject == "Agenda")
            .collect();
        assert_eq!(sent.len(), 1, "exactly one Sent copy");
        assert_eq!(store.account_status(&account.id).unwrap().state, SyncState::UpToDate);
    }

    #[test]
    fn cancelled_send_is_not_sent() {
        let (store, account, provider) = seeded();
        let draft = queued_draft(&store, &account);
        store.cancel_send(&draft.id).unwrap();
        provider.set_online(true);
        let outcome = block_on(run_sync(store.clone(), provider, account.clone()));
        assert!(outcome.sent.is_empty());
        assert_eq!(store.drafts(&account.id, false).unwrap().len(), 1);
    }
}
