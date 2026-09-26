-- Telemetria de retenção: o que cada capitão fez e quando. Só leitura de
-- análise (D1/D7, metas cumpridas, mecânicas usadas); o jogo nunca lê.
-- Sem FK: telemetria não bloqueia nada nem segura personagem.
CREATE TABLE captain_events (
    id BIGSERIAL PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    character_id UUID NOT NULL,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_captain_events_kind_at ON captain_events (kind, at);
CREATE INDEX idx_captain_events_character_at ON captain_events (character_id, at);
