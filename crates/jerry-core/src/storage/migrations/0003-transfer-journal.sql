-- #50. Every copy and move, written down before it starts, so that one Jerry didn't finish is
-- found again at the next start (Tech PRD «Crash recovery»).
--
-- `run` is the Jerry that queued it (its start time, in ms): a job of another run that isn't
-- ended is one that run didn't finish. `state` is 'queued' (never started), 'running' (a turn
-- began; paused and waiting jobs are running too) or 'ended'. `sources` holds the paths one after
-- another, each ended by a 0 unit; every path is a BLOB, the exact long form as UTF-16LE.
-- `writing_source` and `writing_target` are the last file the job began to write, and `part` its
-- temporary file (`name.ext.jerrypart`): the one file a job can leave half written, and the only
-- one Jerry removes without asking (data safety rule 7). Ended jobs go after 7 days.
CREATE TABLE transfer_journal (
    id INTEGER PRIMARY KEY,
    run INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('copy', 'move')),
    sources BLOB NOT NULL,
    destination BLOB NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'ended')),
    files_done INTEGER NOT NULL DEFAULT 0,
    files_total INTEGER NOT NULL DEFAULT 0,
    bytes_done INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    writing_source BLOB,
    writing_target BLOB,
    part BLOB,
    updated INTEGER NOT NULL
) STRICT;

CREATE INDEX transfer_journal_by_state ON transfer_journal (state, updated);
