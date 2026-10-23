use super::codec;
use jiff::tz::TimeZone;
use overlook_core::*;

fn account() -> Account {
    Account {
        id: AccountId::from("test"),
        provider: ProviderKind::Imap,
        display_name: "Test".into(),
        email: "me@example.test".into(),
        created_at: 0,
    }
}
fn calendar() -> Calendar {
    Calendar {
        id: CalendarId::from("caldav:test:calendar"),
        account_id: AccountId::from("test"),
        provider_id: None,
        name: "Test".into(),
        color: None,
        is_primary: true,
        can_edit: false,
        visible: true,
        sort_order: 0,
        default_reminders: vec![],
    }
}
fn document(events: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Overlook tests//EN\r\n{events}END:VCALENDAR\r\n")
}
fn event(properties: &str) -> String {
    format!("BEGIN:VEVENT\r\nUID:test-uid\r\n{properties}\r\nEND:VEVENT\r\n")
}
fn parse(body: &str, zone: &str) -> Result<Vec<Event>, super::CalDavError> {
    codec::events(body, &account(), &calendar(), &TimeZone::get(zone).unwrap())
}

#[test]
fn reads_folded_escaped_unicode_text_quoted_attendees_and_display_alarms() {
    let body = document(&event("DTSTART:20261022T160000Z\r\nDTEND:20261022T170000Z\r\nSUMMARY:Meet\\, café\r\n  team\r\nDESCRIPTION:First\\nSecond\\; third\\\\end\r\nORGANIZER;CN=Host:mailto:host@example.test\r\nATTENDEE;CN=\"Doe, Jane\";PARTSTAT=ACCEPTED:mailto:me@example.test\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-PT30M\r\nEND:VALARM"));
    let events = parse(&body, "America/Phoenix").unwrap();
    let e = &events[0];
    assert_eq!(e.summary, "Meet, café team");
    assert_eq!(e.description.as_deref(), Some("First\nSecond; third\\end"));
    assert_eq!(e.duration_minutes(), 60);
    assert_eq!(e.attendees[0].name.as_deref(), Some("Doe, Jane"));
    assert_eq!(e.my_response(), Some(ResponseStatus::Accepted));
    assert_eq!(e.reminders, Some(vec![15]));
}

#[test]
fn dates_keep_exclusive_end_across_dst_and_missing_end_means_one_day() {
    let body = document(&event(
        "DTSTART;VALUE=DATE:20260307\r\nDTEND;VALUE=DATE:20260310",
    ));
    let e = parse(&body, "America/New_York").unwrap().remove(0);
    assert!(e.all_day);
    assert_eq!(e.end - e.start, 71 * 3_600_000);
    let e = parse(
        &document(&event("DTSTART;VALUE=DATE:20260308")),
        "America/New_York",
    )
    .unwrap()
    .remove(0);
    assert_eq!(e.end - e.start, 23 * 3_600_000);
}

#[test]
fn floating_and_iana_times_use_the_correct_zone_and_duration() {
    let floating = document(&event("DTSTART:20261022T090000\r\nDURATION:PT90M"));
    let e = parse(&floating, "America/Phoenix").unwrap().remove(0);
    assert_eq!(e.start, tz::parse_rfc3339("2026-10-22T16:00:00Z").unwrap());
    assert_eq!(e.duration_minutes(), 90);
    let zoned = document(&event(
        "DTSTART;TZID=America/New_York:20261022T090000\r\nDURATION:PT1H",
    ));
    assert_eq!(
        parse(&zoned, "America/Phoenix").unwrap()[0].start,
        tz::parse_rfc3339("2026-10-22T13:00:00Z").unwrap()
    );
}

#[test]
fn moved_occurrence_keeps_identity_and_cancelled_occurrence_is_omitted() {
    let before = document(&event(
        "RECURRENCE-ID:20261022T160000Z\r\nDTSTART:20261022T160000Z\r\nDURATION:PT1H",
    ));
    let moved = before.replace("DTSTART:20261022T160000Z", "DTSTART:20261023T170000Z");
    let a = parse(&before, "UTC").unwrap().remove(0);
    let b = parse(&moved, "UTC").unwrap().remove(0);
    assert_eq!(a.id, b.id);
    assert_ne!(a.start, b.start);
    assert_eq!(b.original_start, Some(a.start));
    let cancelled = document(&event("RECURRENCE-ID:20261022T160000Z\r\nSTATUS:CANCELLED"));
    assert!(parse(&cancelled, "UTC").unwrap().is_empty());
}

#[test]
fn rejects_unexpanded_ambiguous_incomplete_or_invalid_data() {
    for props in [
        "DTSTART:20261022T160000Z\r\nRRULE:FREQ=DAILY",
        "DTSTART:20261022T160000Z\r\nDTSTART:20261023T160000Z",
        "DTSTART;TZID=Custom/Unknown:20261022T160000",
        "DTSTART;VALUE=DATE:20260230",
        "DTSTART:20261022T160000Z\r\nDTEND:20261021T160000Z",
        "DTSTART:20261022T160000Z\r\nDURATION:P1M",
    ] {
        assert!(parse(&document(&event(props)), "UTC").is_err(), "{props}");
    }
    assert!(parse(
        &document(&event("DTSTART:20261022T160000Z")).replace("END:VEVENT\r\n", ""),
        "UTC"
    )
    .is_err());
    assert!(parse(&(document("") + &document("")), "UTC").is_err());
}

#[test]
fn separate_calendars_do_not_collide_and_duplicate_instances_fail() {
    let body = document(&event("DTSTART:20261022T160000Z"));
    let a = parse(&body, "UTC").unwrap();
    let mut other = calendar();
    other.id = CalendarId::from("caldav:other:calendar");
    let b = codec::events(&body, &account(), &other, &TimeZone::UTC).unwrap();
    assert_ne!(a[0].id, b[0].id);
    assert!(parse(
        &document(&(event("DTSTART:20261022T160000Z") + &event("DTSTART:20261022T160000Z"))),
        "UTC"
    )
    .is_err());
}
