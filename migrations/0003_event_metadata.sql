ALTER TABLE events ADD COLUMN metadata jsonb NOT NULL DEFAULT '{"calendarId":"personal","presentation":"standard","durationMinute":null}';
