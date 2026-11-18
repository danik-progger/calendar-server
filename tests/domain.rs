use calendar_server::domain::Event;
use chrono::{DateTime, Utc};
use uuid::Uuid;

fn time(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}

fn event() -> Event {
    Event::new(
        Uuid::new_v4(),
        "  Meeting  ".into(),
        Some("Notes".into()),
        time("2026-11-13T10:00:00Z"),
        time("2026-11-13T11:00:00Z"),
        false,
    )
    .unwrap()
}

#[test]
fn construction_enforces_invariants_without_http_or_storage() {
    assert_eq!(event().title(), "  Meeting  ");
    for end in [
        "2026-11-13T09:00:00Z",
        "2026-11-13T10:00:00Z",
        "2026-11-13T10:00:00.000000001Z",
    ] {
        assert!(
            Event::new(
                Uuid::new_v4(),
                "Meeting".into(),
                None,
                time("2026-11-13T10:00:00Z"),
                time(end),
                false
            )
            .is_err()
        );
    }
}

#[test]
fn minimum_duration_matches_storage_precision() {
    for (start, end, valid) in [
        (
            "1999-12-31T23:59:30.000000100Z",
            "2000-01-01T00:00:30.000000100Z",
            false,
        ),
        (
            "1999-12-31T23:59:30.000001Z",
            "2000-01-01T00:00:30.000001Z",
            true,
        ),
    ] {
        let result = Event::new(
            Uuid::new_v4(),
            "Meeting".into(),
            None,
            time(start),
            time(end),
            false,
        );
        assert_eq!(result.is_ok(), valid, "{start} to {end}");
    }
}

#[test]
fn invalid_update_preserves_entire_event() {
    let mut event = event();
    let original = event.clone();
    assert!(
        event
            .update(
                "Changed".into(),
                None,
                event.end_at(),
                event.start_at(),
                true
            )
            .is_err()
    );
    assert_eq!(event.title(), original.title());
    assert_eq!(event.description(), original.description());
    assert_eq!(event.start_at(), original.start_at());
    assert_eq!(event.end_at(), original.end_at());
    assert_eq!(event.all_day(), original.all_day());
}

#[test]
fn valid_update_changes_details() {
    let mut event = event();
    let start = time("2026-11-14T00:00:00Z");
    let end = time("2026-11-15T00:00:00Z");
    event
        .update("  Holiday  ".into(), None, start, end, true)
        .unwrap();
    assert_eq!(event.title(), "  Holiday  ");
    assert_eq!(event.description(), None);
    assert_eq!((event.start_at(), event.end_at()), (start, end));
    assert!(event.all_day());
}

#[test]
fn nonempty_text_is_preserved() {
    for title in ["  ", "title\0"] {
        let mut event = Event::new(
            Uuid::new_v4(),
            title.into(),
            Some("notes\0".into()),
            time("2026-11-13T10:00:00Z"),
            time("2026-11-13T11:00:00Z"),
            false,
        )
        .unwrap();
        assert_eq!(event.title(), title);
        assert_eq!(event.description(), Some("notes\0"));
        event
            .update(
                title.into(),
                Some("updated\0".into()),
                event.start_at(),
                event.end_at(),
                false,
            )
            .unwrap();
        assert_eq!(event.title(), title);
        assert_eq!(event.description(), Some("updated\0"));
    }
}

#[test]
fn empty_title_is_rejected_on_creation_and_update() {
    let mut event = event();
    assert!(
        Event::new(
            Uuid::new_v4(),
            String::new(),
            None,
            event.start_at(),
            event.end_at(),
            false
        )
        .is_err()
    );
    let original = event.clone();
    assert!(
        event
            .update(String::new(), None, event.start_at(), event.end_at(), true)
            .is_err()
    );
    assert_eq!(event.title(), original.title());
    assert_eq!(event.description(), original.description());
    assert_eq!(event.all_day(), original.all_day());
}
