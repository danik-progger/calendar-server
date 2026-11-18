use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use uuid::Uuid;

/// A scheduled item with an exclusive end and a stable identity.
#[derive(Clone, Debug)]
pub struct Event {
    id: Uuid,
    title: String,
    description: Option<String>,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    all_day: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InvalidEvent {
    EmptyTitle,
    InvalidInterval,
}

impl std::fmt::Display for InvalidEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("Event title must not be empty"),
            Self::InvalidInterval => {
                f.write_str("Event must last at least one minute at storage precision")
            }
        }
    }
}

impl std::error::Error for InvalidEvent {}

impl Event {
    /// Creates an event with a nonempty title and a duration of at least one minute.
    pub fn new(
        id: Uuid,
        title: String,
        description: Option<String>,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        all_day: bool,
    ) -> Result<Self, InvalidEvent> {
        if title.is_empty() {
            return Err(InvalidEvent::EmptyTitle);
        }
        const MIN_DURATION: TimeDelta = TimeDelta::minutes(1);
        if end_at.signed_duration_since(start_at) < MIN_DURATION {
            return Err(InvalidEvent::InvalidInterval);
        }
        // SQLx truncates to microseconds relative to PostgreSQL's epoch.
        const EPOCH: NaiveDate = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        let epoch = EPOCH.and_hms_opt(0, 0, 0).unwrap();
        let start = (start_at.naive_utc() - epoch)
            .num_microseconds()
            .ok_or(InvalidEvent::InvalidInterval)?;
        let end = (end_at.naive_utc() - epoch)
            .num_microseconds()
            .ok_or(InvalidEvent::InvalidInterval)?;
        if end.saturating_sub(start) < MIN_DURATION.num_microseconds().unwrap() {
            return Err(InvalidEvent::InvalidInterval);
        }
        Ok(Self {
            id,
            title,
            description,
            start_at,
            end_at,
            all_day,
        })
    }

    /// Replaces editable fields atomically. An invalid replacement leaves the event intact.
    pub fn update(
        &mut self,
        title: String,
        description: Option<String>,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        all_day: bool,
    ) -> Result<(), InvalidEvent> {
        *self = Self::new(self.id, title, description, start_at, end_at, all_day)?;
        Ok(())
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn start_at(&self) -> DateTime<Utc> {
        self.start_at
    }

    pub fn end_at(&self) -> DateTime<Utc> {
        self.end_at
    }

    pub fn all_day(&self) -> bool {
        self.all_day
    }
}
