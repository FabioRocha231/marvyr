-- Ofertas de escambo não têm preço: ganham 1 para o NOT NULL voltar.
UPDATE market_orders SET unit_price = 1 WHERE unit_price IS NULL;
ALTER TABLE market_orders
    ALTER COLUMN unit_price SET NOT NULL,
    DROP COLUMN IF EXISTS ask_quantity,
    DROP COLUMN IF EXISTS ask_item_definition_id;
