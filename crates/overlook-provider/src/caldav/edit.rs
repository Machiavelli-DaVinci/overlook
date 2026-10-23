//! Lossless property patching: unknown properties, alarms, and timezone components
//! survive edits. Expanded display events are never serialized as whole resources.
use super::CalDavError;
use ical::property::Property;
use overlook_core::*;

const BAD: CalDavError = CalDavError::InvalidResponse(
    "This calendar resource cannot be edited safely. No change was uploaded.",
);
#[derive(Clone)]
enum Item {
    Property(String),
    Component(Component),
}
#[derive(Clone)]
struct Component {
    name: String,
    items: Vec<Item>,
}

fn property(raw: &str) -> Result<Property, CalDavError> {
    ical::PropertyParser::from_reader(raw.as_bytes())
        .next()
        .ok_or(BAD)?
        .map_err(|_| BAD)
}
fn name(raw: &str) -> String {
    property(raw)
        .map(|p| p.name.to_ascii_uppercase())
        .unwrap_or_default()
}
impl Component {
    fn get(&self, key: &str) -> Result<Option<Property>, CalDavError> {
        let mut values = self.items.iter().filter_map(|i| match i {
            Item::Property(s) if name(s) == key => Some(property(s)),
            _ => None,
        });
        let value = values.next().transpose()?;
        if values.next().is_some() {
            return Err(BAD);
        }
        Ok(value)
    }
    fn value(&self, key: &str) -> Result<Option<String>, CalDavError> {
        Ok(self.get(key)?.and_then(|p| p.value))
    }
    fn remove(&mut self, key: &str) {
        self.items
            .retain(|i| !matches!(i, Item::Property(s) if name(s)==key));
    }
    fn set(&mut self, key: &str, line: String) {
        self.remove(key);
        self.items.push(Item::Property(fold(&line)));
    }
    fn render(&self) -> String {
        let mut s = format!("BEGIN:{}\r\n", self.name);
        for item in &self.items {
            match item {
                Item::Property(p) => {
                    s.push_str(p);
                    s.push_str("\r\n");
                }
                Item::Component(c) => s.push_str(&c.render()),
            }
        }
        s.push_str(&format!("END:{}\r\n", self.name));
        s
    }
    fn canonical(&self) -> String {
        let mut parts = self
            .items
            .iter()
            .map(|i| match i {
                Item::Property(p) => p.replace("\r\n ", "").replace("\r\n\t", ""),
                Item::Component(c) => c.canonical(),
            })
            .collect::<Vec<_>>();
        parts.sort();
        format!(
            "BEGIN:{}\n{}\nEND:{}",
            self.name,
            parts.join("\n"),
            self.name
        )
    }
}
fn parse(body: &str) -> Result<Component, CalDavError> {
    if body.len() > 2 * 1024 * 1024 || body.contains('\0') {
        return Err(BAD);
    }
    let normalized = body.replace("\r\n", "\n");
    if normalized.contains('\r') {
        return Err(BAD);
    }
    let mut lines: Vec<String> = Vec::new();
    for line in normalized.lines() {
        if line.starts_with([' ', '\t']) {
            lines
                .last_mut()
                .ok_or(BAD)?
                .push_str(&format!("\r\n{line}"));
        } else if !line.is_empty() {
            lines.push(line.into());
        }
    }
    if lines.len() > 50_000 {
        return Err(BAD);
    }
    let mut stack: Vec<Component> = Vec::new();
    let mut root = None;
    for raw in lines {
        let p = property(&raw)?;
        match p.name.to_ascii_uppercase().as_str() {
            "BEGIN" => {
                if root.is_some() || stack.len() >= 8 {
                    return Err(BAD);
                }
                stack.push(Component {
                    name: p.value.ok_or(BAD)?.to_ascii_uppercase(),
                    items: vec![],
                });
            }
            "END" => {
                let c = stack.pop().ok_or(BAD)?;
                if Some(c.name.clone()) != p.value.map(|v| v.to_ascii_uppercase()) {
                    return Err(BAD);
                }
                if let Some(parent) = stack.last_mut() {
                    parent.items.push(Item::Component(c));
                } else {
                    root = Some(c);
                }
            }
            _ => stack.last_mut().ok_or(BAD)?.items.push(Item::Property(raw)),
        }
    }
    let root = root.ok_or(BAD)?;
    if !stack.is_empty()
        || root.name != "VCALENDAR"
        || root.value("VERSION")?.as_deref() != Some("2.0")
        || root.get("METHOD")?.is_some()
    {
        return Err(BAD);
    }
    Ok(root)
}
fn fold(s: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in s.chars() {
        if width + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out
}
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
fn date_line(key: &str, event: &Event, end: bool) -> String {
    let at = if end { event.end } else { event.start };
    if event.all_day {
        format!("{key};VALUE=DATE:{}", tz::local_date(at).strftime("%Y%m%d"))
    } else {
        format!("{key}:{}", tz::utc_basic(at))
    }
}
fn patch(component: &mut Component, edit: &CalDavEdit) -> Result<(), CalDavError> {
    if let Some(after) = &edit.after {
        if after.end <= after.start
            || jiff::Timestamp::from_millisecond(after.start).is_err()
            || jiff::Timestamp::from_millisecond(after.end).is_err()
            || after.wants_conference
            || after.attendees.iter().any(|a| !a.is_self)
        {
            return Err(CalDavError::InvalidConfiguration(
                "CalDAV changes currently support personal events without invitations.",
            ));
        }
        let before = edit.before.as_ref();
        for (key, value, changed) in [
            (
                "SUMMARY",
                Some(after.summary.as_str()),
                before.is_none_or(|e| e.summary != after.summary),
            ),
            (
                "DESCRIPTION",
                after.description.as_deref(),
                before.is_none_or(|e| e.description != after.description),
            ),
            (
                "LOCATION",
                after.location.as_deref(),
                before.is_none_or(|e| e.location != after.location),
            ),
        ] {
            if changed {
                component.remove(key);
                if let Some(value) = value {
                    component.set(key, format!("{key}:{}", escape(value)));
                }
            }
        }
        if before.is_none_or(|e| {
            e.start != after.start || e.end != after.end || e.all_day != after.all_day
        }) {
            component.remove("DURATION");
            component.set("DTSTART", date_line("DTSTART", after, false));
            component.set("DTEND", date_line("DTEND", after, true));
        }
        if before.is_none() {
            component.set("STATUS", "STATUS:CONFIRMED".into());
        }
    } else {
        component.set("STATUS", "STATUS:CANCELLED".into());
    }
    let sequence = component
        .value("SEQUENCE")?
        .map(|v| v.parse::<u32>().map_err(|_| BAD))
        .transpose()?
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(BAD)?;
    component.set("SEQUENCE", format!("SEQUENCE:{sequence}"));
    component.set("DTSTAMP", format!("DTSTAMP:{}", tz::utc_basic(time::now())));
    component.set(
        "LAST-MODIFIED",
        format!("LAST-MODIFIED:{}", tz::utc_basic(time::now())),
    );
    Ok(())
}

/// Only supplied form fields change. Series rules, custom properties and nested
/// components remain intact. Following-series splits are deliberately not guessed.
pub fn prepare_resource(
    base: Option<&str>,
    edits: &[CalDavEdit],
    owner: &str,
) -> Result<Option<String>, CalDavError> {
    if edits.is_empty() || edits.len() > 128 {
        return Err(BAD);
    }
    let mut root = match base {
        Some(s) => parse(s)?,
        None => Component {
            name: "VCALENDAR".into(),
            items: vec![
                Item::Property("VERSION:2.0".into()),
                Item::Property("PRODID:-//Last Refuge//Overlook//EN".into()),
            ],
        },
    };
    for edit in edits {
        let event = edit.after.as_ref().or(edit.before.as_ref()).ok_or(BAD)?;
        let uid = event
            .ical_uid
            .as_ref()
            .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
            .ok_or(BAD)?;
        if edit.scope == RecurrenceScope::Following {
            return Err(CalDavError::InvalidConfiguration("This and following changes are not supported for CalDAV yet. Choose one occurrence."));
        }
        let mut indices = Vec::new();
        let mut master = None;
        for (i, item) in root.items.iter().enumerate() {
            if let Item::Component(c) = item {
                if c.name == "VEVENT" {
                    if c.get("RECURRENCE-ID")?
                        .and_then(|p| p.params)
                        .is_some_and(|params| {
                            params
                                .iter()
                                .any(|(key, _)| key.eq_ignore_ascii_case("RANGE"))
                        })
                        && !(edit.after.is_none() && edit.scope == RecurrenceScope::All)
                    {
                        return Err(CalDavError::InvalidConfiguration("This series contains a following-occurrences exception and cannot be edited safely yet."));
                    }
                    if c.value("UID")?.as_ref() != Some(uid) || c.get("ATTENDEE")?.is_some() {
                        return Err(CalDavError::InvalidConfiguration("Meetings and resources containing multiple event identities remain read-only."));
                    }
                    if c.value("ORGANIZER")?
                        .is_some_and(|o| !o.eq_ignore_ascii_case(&format!("mailto:{owner}")))
                    {
                        return Err(BAD);
                    }
                    if c.get("RECURRENCE-ID")?.is_none() && master.replace(i).is_some() {
                        return Err(BAD);
                    }
                    indices.push(i);
                }
            }
        }
        if edit.before.is_none() {
            if let Some(index) = master {
                let Item::Component(existing) = &mut root.items[index] else {
                    return Err(BAD);
                };
                if indices.len() != 1
                    || existing.get("RRULE")?.is_some()
                    || existing.get("RDATE")?.is_some()
                {
                    return Err(BAD);
                }
                patch(existing, edit)?;
                continue;
            }
            if !indices.is_empty() {
                return Err(BAD);
            }
            let mut event_node = Component {
                name: "VEVENT".into(),
                items: vec![Item::Property(format!("UID:{}", escape(uid)))],
            };
            patch(&mut event_node, edit)?;
            root.items.push(Item::Component(event_node));
            continue;
        }
        let master = master.ok_or(BAD)?;
        let Item::Component(master_node) = &root.items[master] else {
            return Err(BAD);
        };
        let recurring = master_node.get("RRULE")?.is_some()
            || master_node.get("RDATE")?.is_some()
            || indices.len() > 1;
        if edit.after.is_none() && (!recurring || edit.scope == RecurrenceScope::All) {
            return Ok(None);
        }
        if recurring && edit.scope != RecurrenceScope::This {
            return Err(CalDavError::InvalidConfiguration(
                "Edit one CalDAV occurrence at a time. Series-wide edits are not enabled yet.",
            ));
        }
        let index = if recurring {
            let before = edit.before.as_ref().ok_or(BAD)?;
            let slot = before.original_start.unwrap_or(before.start);
            let mut found = None;
            for i in &indices {
                if let Item::Component(c) = &root.items[*i] {
                    if let Some(p) = c.get("RECURRENCE-ID")? {
                        if super::codec::property_timestamp(&p)? == slot
                            && found.replace(*i).is_some()
                        {
                            return Err(BAD);
                        }
                    }
                }
            }
            match found {
                Some(i) => i,
                None => {
                    let mut exception = master_node.clone();
                    for key in ["RRULE", "RDATE", "EXDATE", "EXRULE"] {
                        exception.remove(key);
                    }
                    // A title-only edit of a later occurrence must not inherit the
                    // master's first date. Materialize this instance before patching.
                    exception.remove("DURATION");
                    exception.set("DTSTART", date_line("DTSTART", before, false));
                    exception.set("DTEND", date_line("DTEND", before, true));
                    // Preserve DATE vs floating/TZID/UTC semantics of the series identity.
                    let start = master_node.get("DTSTART")?.ok_or(BAD)?;
                    let value_type = start
                        .params
                        .as_ref()
                        .and_then(|p| p.iter().find(|(k, _)| k.eq_ignore_ascii_case("VALUE")))
                        .and_then(|(_, v)| v.first());
                    let tzid = start
                        .params
                        .as_ref()
                        .and_then(|p| p.iter().find(|(k, _)| k.eq_ignore_ascii_case("TZID")))
                        .and_then(|(_, v)| v.first());
                    let rid = if value_type.is_some_and(|v| v == "DATE") {
                        format!(
                            "RECURRENCE-ID;VALUE=DATE:{}",
                            tz::local_date(slot).strftime("%Y%m%d")
                        )
                    } else if let Some(id) = tzid {
                        let zone=jiff::tz::TimeZone::get(id).map_err(|_|CalDavError::InvalidConfiguration("This series uses a custom timezone. Edit it in the provider's calendar."))?;
                        format!(
                            "RECURRENCE-ID;TZID={id}:{}",
                            jiff::Timestamp::from_millisecond(slot)
                                .map_err(|_| BAD)?
                                .to_zoned(zone)
                                .strftime("%Y%m%dT%H%M%S")
                        )
                    } else if start.value.as_ref().is_some_and(|s| s.ends_with('Z')) {
                        format!("RECURRENCE-ID:{}", tz::utc_basic(slot))
                    } else {
                        format!(
                            "RECURRENCE-ID:{}",
                            tz::zoned(slot).strftime("%Y%m%dT%H%M%S")
                        )
                    };
                    exception.set("RECURRENCE-ID", rid);
                    // A cloned master must use this occurrence's times even for a text-only edit.
                    exception.remove("DURATION");
                    exception.set("DTSTART", date_line("DTSTART", before, false));
                    exception.set("DTEND", date_line("DTEND", before, true));
                    root.items.push(Item::Component(exception));
                    root.items.len() - 1
                }
            }
        } else {
            master
        };
        let Item::Component(target) = &mut root.items[index] else {
            return Err(BAD);
        };
        patch(target, edit)?;
    }
    let result = root.render();
    if result.len() > 2 * 1024 * 1024 {
        return Err(BAD);
    }
    Ok(Some(result))
}

/// Conservative reconciliation: tolerate folding, line endings and property order,
/// but never acknowledge a different resource merely because its UID matches.
pub fn resource_equivalent(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Ok(a), Ok(b)) => a.canonical() == b.canonical(),
        _ => false,
    }
}

pub fn resource_preview(body: &str) -> String {
    let Ok(root) = parse(body) else {
        return "Provider version is available but cannot be previewed. Your local changes are saved.".into();
    };
    let mut previews = Vec::new();
    for item in root.items {
        if let Item::Component(c) = item {
            if c.name == "VEVENT" {
                let title = c
                    .value("SUMMARY")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "Untitled event".into());
                let at = c
                    .get("DTSTART")
                    .ok()
                    .flatten()
                    .and_then(|p| super::codec::property_timestamp(&p).ok());
                let mut text = format!("Provider: {title}");
                if let Some(at) = at {
                    text.push_str(&format!(
                        " · {} {}",
                        tz::format_day(at),
                        tz::format_clock(at)
                    ));
                }
                for key in ["LOCATION", "DESCRIPTION"] {
                    if let Ok(Some(value)) = c.value(key) {
                        text.push_str(&format!(" · {}", value.replace("\\n", " ")));
                    }
                }
                if c.get("RRULE").ok().flatten().is_some() {
                    text.push_str(" (series)");
                }
                previews.push(text.chars().take(650).collect::<String>());
                if previews.len() == 3 {
                    break;
                }
            }
        }
    }
    previews.join("\n")
}

/// Reapply only the user's fields. A changed recurring schedule needs a fresh
/// server-expanded instance; retaining old occurrence times would lose remote edits.
pub fn prepare_conflict_resource(
    base: Option<&str>,
    remote: &str,
    edits: &[CalDavEdit],
    owner: &str,
) -> Result<Option<String>, CalDavError> {
    fn schedule(body: &str) -> Result<Option<Vec<String>>, CalDavError> {
        let root = parse(body)?;
        let mut recurring = false;
        let mut fields = Vec::new();
        for item in root.items {
            if let Item::Component(c) = item {
                if c.name == "VTIMEZONE" {
                    fields.push(c.canonical());
                }
                if c.name == "VEVENT" && c.get("RECURRENCE-ID")?.is_none() {
                    recurring = c.get("RRULE")?.is_some() || c.get("RDATE")?.is_some();
                    for item in &c.items {
                        if let Item::Property(line) = item {
                            let p = property(line)?;
                            if [
                                "DTSTART", "DTEND", "DURATION", "RRULE", "RDATE", "EXDATE",
                                "EXRULE",
                            ]
                            .contains(&p.name.to_ascii_uppercase().as_str())
                            {
                                fields.push(line.clone());
                            }
                        }
                    }
                }
            }
        }
        fields.sort();
        Ok(recurring.then_some(fields))
    }
    if let Some(base) = base {
        let before = schedule(base)?;
        let after = schedule(remote)?;
        let deleting_series = edits
            .last()
            .is_some_and(|e| e.after.is_none() && e.scope == RecurrenceScope::All);
        if !deleting_series && (before.is_some() || after.is_some()) && before != after {
            return Err(CalDavError::InvalidConfiguration("The repeating schedule changed at the provider. Use the provider version, sync, and edit the refreshed occurrence to avoid overwriting its new time."));
        }
    }
    prepare_resource(Some(remote), edits, owner)
}
