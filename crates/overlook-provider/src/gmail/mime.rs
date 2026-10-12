//! Builds the RFC 822 message for `users.messages.send`.

use lettre::message::header::ContentType;
use lettre::message::{Attachment as MimeAttachment, Mailbox, MultiPart, SinglePart};
use lettre::Message as MimeMessage;
use overlook_core::*;

fn mailbox(address: &Address) -> Result<Mailbox, String> {
    let email = address.email.parse().map_err(|e| format!("invalid address “{}”: {e}", address.email))?;
    Ok(Mailbox::new(address.name.clone().filter(|n| !n.trim().is_empty()), email))
}

/// Returns the raw message bytes. `rfc_id` is the Message-ID (with angle brackets).
pub fn build_raw(account: &Account, draft: &Draft, in_reply_to: Option<&Message>, rfc_id: &str) -> Result<Vec<u8>, String> {
    let mut builder = MimeMessage::builder()
        .from(mailbox(&Address::named(&account.display_name, &account.email))?)
        .subject(draft.subject.clone())
        .message_id(Some(rfc_id.to_owned()))
        .user_agent("Overlook".to_owned());
    for a in &draft.to {
        builder = builder.to(mailbox(a)?);
    }
    for a in &draft.cc {
        builder = builder.cc(mailbox(a)?);
    }
    for a in &draft.bcc {
        builder = builder.bcc(mailbox(a)?);
    }
    if let Some(source) = in_reply_to {
        if let Some(id) = &source.rfc_message_id {
            builder = builder.in_reply_to(id.clone());
            let references = match &source.in_reply_to {
                Some(parent) => format!("{parent} {id}"),
                None => id.clone(),
            };
            builder = builder.references(references);
        }
    }
    let text = if draft.body_text.is_empty() { "\n".to_owned() } else { draft.body_text.clone() };
    let message = if draft.attachments.is_empty() {
        builder.header(ContentType::TEXT_PLAIN).body(text)
    } else {
        let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(text));
        for att in &draft.attachments {
            let bytes = std::fs::read(&att.local_path).map_err(|e| format!("could not read attachment {}: {e}", att.filename))?;
            let content_type = ContentType::parse(&att.mime_type).unwrap_or(ContentType::parse("application/octet-stream").unwrap());
            multipart = multipart.singlepart(MimeAttachment::new(att.filename.clone()).body(bytes, content_type));
        }
        builder.multipart(multipart)
    }
    .map_err(|e| e.to_string())?;
    Ok(message.formatted())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_plain_message_with_threading() {
        let account = Account {
            id: AccountId::from("g1"),
            provider: ProviderKind::Gmail,
            display_name: "Jason Alexander".into(),
            email: "jason@example.com".into(),
            created_at: 0,
        };
        let mut draft = Draft::new(account.id.clone(), DraftKind::Reply, 0);
        draft.to = vec![Address::named("Maya Chen", "maya@northstar.example")];
        draft.subject = "Re: Launch créative".into();
        draft.body_text = "Looks great — thanks!".into();
        let mut source = Message {
            id: MessageId::from("g1:1"),
            account_id: account.id.clone(),
            conversation_id: ConversationId::from("g1:t"),
            provider_id: None,
            from: Address::named("Maya Chen", "maya@northstar.example"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            reply_to: None,
            subject: "Launch créative".into(),
            snippet: String::new(),
            body: Body::default(),
            date: 0,
            is_read: true,
            is_starred: false,
            is_outgoing: false,
            label_ids: Default::default(),
            attachments: vec![],
            rfc_message_id: Some("<abc@northstar.example>".into()),
            in_reply_to: None,
            category: None,
            is_important: false,
            focus_override: None,
        };
        source.in_reply_to = Some("<root@northstar.example>".into());
        let raw = String::from_utf8(build_raw(&account, &draft, Some(&source), "<d1@overlook.lastrefuge.ai>").unwrap()).unwrap();
        assert!(raw.contains("From: \"Jason Alexander\" <jason@example.com>"), "{raw}");
        assert!(raw.contains("To: \"Maya Chen\" <maya@northstar.example>"));
        assert!(raw.contains("Message-ID: <d1@overlook.lastrefuge.ai>"));
        assert!(raw.contains("In-Reply-To: <abc@northstar.example>"));
        assert!(raw.contains("References: <root@northstar.example> <abc@northstar.example>"));
        assert!(raw.contains("Subject: Re: Launch =?utf-8?b?"), "{raw}");
        assert!(raw.contains("Content-Type: text/plain"));
    }
}
