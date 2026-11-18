-- Legacy events stay unassigned until an operator explicitly assigns their owner.
ALTER TABLE events ADD COLUMN owner_id text;
CREATE INDEX events_owner_start ON events (owner_id, start_at, id);
