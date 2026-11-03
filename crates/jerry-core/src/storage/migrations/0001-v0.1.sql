-- v0.1 (#21). Paths are BLOBs: the exact path as UTF-16LE, Windows' own form, so a name that
-- isn't valid Unicode survives too. Times are milliseconds since the Unix epoch.

-- Setting key (camelCase, as on the wire) → JSON value. Only keys the user changed have a row.
CREATE TABLE settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

-- Sidebar favorite groups, in sidebar order.
CREATE TABLE favorite_groups (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    position INTEGER NOT NULL
) STRICT;

-- Sidebar favorites. No group: the top level. A name replaces the folder's own.
CREATE TABLE favorites (
    id INTEGER PRIMARY KEY,
    path BLOB NOT NULL,
    name TEXT,
    group_id INTEGER REFERENCES favorite_groups (id),
    position INTEGER NOT NULL
) STRICT;

CREATE INDEX favorites_by_group ON favorites (group_id, position);

-- Recently opened folders; at most 500, the oldest go.
CREATE TABLE recent_folders (
    path BLOB PRIMARY KEY NOT NULL,
    last_opened INTEGER NOT NULL,
    open_count INTEGER NOT NULL
) STRICT;

CREATE INDEX recent_folders_by_last_opened ON recent_folders (last_opened);

-- The view each folder was last left in; at most 5 000, the least recently used go. `sort` and
-- `columns` are JSON.
CREATE TABLE folder_views (
    path BLOB PRIMARY KEY NOT NULL,
    view TEXT NOT NULL,
    sort TEXT NOT NULL,
    columns TEXT NOT NULL,
    last_used INTEGER NOT NULL
) STRICT;

CREATE INDEX folder_views_by_last_used ON folder_views (last_used);

-- The session: windows, panes, tabs and their history, as JSON. One row.
CREATE TABLE session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    state TEXT NOT NULL,
    saved INTEGER NOT NULL
) STRICT;

-- Shortcuts the user changed: command id → shortcut. NULL: the default shortcut was removed.
CREATE TABLE keybindings (
    command TEXT PRIMARY KEY NOT NULL,
    shortcut TEXT
) STRICT;
