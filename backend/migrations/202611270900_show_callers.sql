-- Members the owner marked as show callers. They may fetch a caller pass for the show, which a
-- session host accepts offline. The owner may always call, whatever this says.
ALTER TABLE show_members ADD COLUMN can_call BOOLEAN NOT NULL DEFAULT FALSE;
