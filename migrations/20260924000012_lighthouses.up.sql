-- v46: faróis erguidos por capitães. O material é consumido na hora (não
-- mora aqui); a linha some quando o farol apaga.
CREATE TABLE lighthouses (
    id BIGINT PRIMARY KEY,
    builder UUID NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    -- Unix (s) em que apaga.
    expires_at BIGINT NOT NULL
);
