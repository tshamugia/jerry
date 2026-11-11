-- #66. The last searches (PRD «ღრმა ძებნა»: the last 20), newest first by `used_at` (ms since the
-- Unix epoch). `query` is the name text as typed, `filters` the rest of the search as JSON
-- (`jerry_core::search::SavedSearch` without its name), so a search run again is one row.
CREATE TABLE search_history (
    query TEXT NOT NULL,
    filters TEXT NOT NULL,
    used_at INTEGER NOT NULL,
    PRIMARY KEY (query, filters)
) STRICT;
