-- v35: progressão do capitão (Diário de Bordo e o que vier junto) em JSON;
-- campo novo entra com default no serde, sem migration por campo.
ALTER TABLE characters ADD COLUMN progress JSONB NOT NULL DEFAULT '{}';
