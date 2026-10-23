//! Recipient-field editing shared by new messages, replies and forwards.

use crate::Address;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// A completion belongs to one field and one text/selection snapshot. Keeping
/// that snapshot prevents a click on an old suggestion from replacing new text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipientQuery {
    pub field: RecipientField,
    value: String,
    selection: Range<usize>,
    token: Range<usize>,
}

impl RecipientQuery {
    /// Input selection offsets are UTF-8 bytes. A selection spanning multiple
    /// recipients must never be replaced by a single completion.
    pub fn new(field: RecipientField, value: &str, selection: Range<usize>) -> Option<Self> {
        if selection.start > selection.end
            || !value.is_char_boundary(selection.start)
            || !value.is_char_boundary(selection.end)
        {
            return None;
        }
        let token = recipient_ranges(value)
            .into_iter()
            .find(|r| r.start <= selection.start && selection.end <= r.end)?;
        let query = Self {
            field,
            value: value.to_owned(),
            selection,
            token,
        };
        (!query.text().is_empty()).then_some(query)
    }

    pub fn text(&self) -> &str {
        self.value[self.token.clone()].trim().trim_matches('"')
    }

    /// Replace only the active recipient, preserving all other addresses and
    /// separators. At the end of a field, leave room to enter another recipient.
    pub fn replacement(
        &self,
        current: &str,
        selection: Range<usize>,
        address: &Address,
    ) -> Option<(Range<usize>, String)> {
        if current != self.value || selection != self.selection {
            return None;
        }
        let part = &current[self.token.clone()];
        let start = self.token.start + part.len() - part.trim_start().len();
        let end = self.token.start + part.trim_end().len();
        let mut header = address.to_header();
        let range = if self.token.end == current.len() {
            header.push_str(", ");
            start..self.token.end
        } else {
            start..end
        };
        Some((range, header))
    }
}

/// Split composer address lists without treating punctuation inside quoted
/// display names or angle addresses as recipient separators. Also used when
/// saving the draft so the completion and parser agree on recipient boundaries.
pub(crate) fn recipient_ranges(input: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut angle = false;
    for (offset, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                ranges.push(start..offset);
                start = offset + ch.len_utf8();
            }
            _ => {}
        }
    }
    ranges.push(start..input.len());
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(
        field: RecipientField,
        value: &str,
        selection: Range<usize>,
        address: &Address,
    ) -> String {
        let query = RecipientQuery::new(field, value, selection.clone()).unwrap();
        assert_eq!(query.field, field);
        let (range, replacement) = query.replacement(value, selection, address).unwrap();
        let mut result = value.to_owned();
        result.replace_range(range, &replacement);
        result
    }

    #[test]
    fn completes_each_recipient_field_after_existing_reply_recipients() {
        for field in [RecipientField::To, RecipientField::Cc, RecipientField::Bcc] {
            let value = "Sender <sender@example.com>, ma";
            let result = complete(
                field,
                value,
                value.len()..value.len(),
                &Address::named("Maya Chen", "maya@example.com"),
            );
            assert_eq!(
                result,
                "Sender <sender@example.com>, Maya Chen <maya@example.com>, "
            );
            assert_eq!(Address::parse_list(&result).len(), 2);
        }
    }

    #[test]
    fn editing_first_or_middle_recipient_preserves_later_recipients() {
        let address = Address::named("Maya", "maya@example.com");
        let value = "ma, last@example.com";
        let query = RecipientQuery::new(RecipientField::To, value, 2..2).unwrap();
        assert_eq!(query.text(), "ma");
        assert_eq!(
            complete(RecipientField::To, value, 2..2, &address),
            "Maya <maya@example.com>, last@example.com"
        );

        let value = "first@example.com; ma ; last@example.com";
        let offset = value.find("ma ;").unwrap();
        assert_eq!(
            complete(RecipientField::Cc, value, offset..offset + 2, &address),
            "first@example.com; Maya <maya@example.com> ; last@example.com"
        );
    }

    #[test]
    fn quoted_names_and_unicode_survive_completion_and_draft_parsing() {
        let existing = Address::named("Doe, Jane \"JJ\"", "jane@example.com");
        let target = Address::named("Élodie; Martin", "elodie@example.com");
        let value = format!("{}, Élo", existing.to_header());
        let query =
            RecipientQuery::new(RecipientField::Bcc, &value, value.len()..value.len()).unwrap();
        assert_eq!(query.text(), "Élo");
        let result = complete(
            RecipientField::Bcc,
            &value,
            value.len()..value.len(),
            &target,
        );
        assert_eq!(Address::parse_list(&result), vec![existing, target]);
    }

    #[test]
    fn rejects_stale_text_or_selection_and_multi_recipient_selections() {
        let value = "ma, jo";
        let query = RecipientQuery::new(RecipientField::To, value, 2..2).unwrap();
        let address = Address::new(None, "maya@example.com");
        assert!(query.replacement("may, jo", 2..2, &address).is_none());
        assert!(query.replacement(value, 6..6, &address).is_none());
        assert!(RecipientQuery::new(RecipientField::To, value, 0..6).is_none());
        assert!(RecipientQuery::new(RecipientField::To, "Élo", 1..1).is_none());
        assert!(RecipientQuery::new(RecipientField::To, value, 99..99).is_none());
    }

    #[test]
    fn blank_tokens_do_not_offer_or_replace_anything() {
        for value in ["", " ", "sender@example.com, ", "sender@example.com;"] {
            assert!(
                RecipientQuery::new(RecipientField::Cc, value, value.len()..value.len()).is_none()
            );
        }
    }

    #[test]
    fn completing_a_quoted_partial_replaces_the_whole_name() {
        let value = "\"Doe, Ja\"";
        let query = RecipientQuery::new(RecipientField::To, value, 8..8).unwrap();
        assert_eq!(query.text(), "Doe, Ja");
        let address = Address::named("Doe, Jane", "jane@example.com");
        let result = complete(RecipientField::To, value, 8..8, &address);
        assert_eq!(Address::parse_list(&result), vec![address]);
    }

    #[test]
    fn punctuation_in_contact_names_round_trips_without_changing_recipients() {
        let addresses: Vec<_> = [
            "Doe, Jane",
            "Maya; Research",
            "Alex \"Ace\" Smith",
            "Team \\ Support",
            "Team <Support>",
            "Help (US)",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, name)| Address::named(name, &format!("contact{i}@example.com")))
        .collect();
        let headers = addresses
            .iter()
            .map(Address::to_header)
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(Address::parse_list(&headers), addresses);
    }
}
