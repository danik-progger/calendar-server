CREATE TABLE events (
    id uuid PRIMARY KEY,
    title text NOT NULL CHECK (btrim(title) <> ''),
    description text,
    start_at timestamptz NOT NULL,
    end_at timestamptz NOT NULL CHECK (end_at > start_at),
    all_day boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX events_start ON events (start_at, id);
