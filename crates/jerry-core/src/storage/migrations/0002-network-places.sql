-- #34. Network locations the user added to the sidebar (`\\server\share`), in sidebar order.
-- The path is a BLOB like every path: the exact long form (`\\?\UNC\server\share`) as UTF-16LE.
CREATE TABLE network_places (
    path BLOB PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL
) STRICT;
