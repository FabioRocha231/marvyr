-- Afixos de equipamento: raridade e afixos da peça fabricada (JSON do
-- `Quality` de domain-items). NULL = Normal / recurso.
ALTER TABLE item_instances ADD COLUMN quality JSONB;
