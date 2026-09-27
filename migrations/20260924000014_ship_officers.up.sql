-- v59: oficiais de bordo (máscara: 1 artilheiro, 2 contramestre, 4 cirurgião).
-- Moram no navio como a tripulação: afundou, foram junto.
ALTER TABLE ship_instances ADD COLUMN officers SMALLINT NOT NULL DEFAULT 0;
