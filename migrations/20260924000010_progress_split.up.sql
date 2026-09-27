-- v45: o que é consultado entre capitães sai do JSON de progresso.
-- Temporada vira coluna indexada (placar); influência de porto vira tabela
-- (Senhor do Porto). O resto (metas, Livro, maestria) fica no JSONB.
ALTER TABLE characters
    ADD COLUMN season BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN season_points BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN crowns BIGINT NOT NULL DEFAULT 0;
CREATE INDEX idx_characters_season_points ON characters (season, season_points DESC);

CREATE TABLE port_influence (
    character_id UUID NOT NULL REFERENCES characters(id) ON DELETE RESTRICT,
    port TEXT NOT NULL,
    week BIGINT NOT NULL,
    points BIGINT NOT NULL,
    PRIMARY KEY (character_id, port, week)
);
CREATE INDEX idx_port_influence_rank ON port_influence (port, week, points DESC);

-- Dados já gravados no JSON migram para o lugar novo e saem de lá.
UPDATE characters SET
    season = COALESCE((progress->>'season')::bigint, 0),
    season_points = COALESCE((progress->>'season_points')::bigint, 0),
    crowns = COALESCE((progress->>'crowns')::bigint, 0);
INSERT INTO port_influence (character_id, port, week, points)
SELECT c.id, i.key, (c.progress->>'influence_week')::bigint, i.value::bigint
FROM characters c, jsonb_each_text(COALESCE(c.progress->'influence', '{}'::jsonb)) AS i
WHERE c.progress ? 'influence_week';
UPDATE characters
SET progress = progress - 'season' - 'season_points' - 'crowns' - 'influence' - 'influence_week';
