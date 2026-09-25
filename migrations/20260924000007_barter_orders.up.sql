-- Sem moeda: a oferta do mercado vira escambo ("dou X por Y"). unit_price
-- fica só para as ofertas antigas, que o servidor cancela no boot
-- (escrow volta ao armazém do vendedor). wallets/ledger_entries deixam de
-- ser escritas, mas ficam no banco como histórico.
ALTER TABLE market_orders
    ALTER COLUMN unit_price DROP NOT NULL,
    ADD COLUMN IF NOT EXISTS ask_item_definition_id UUID,
    ADD COLUMN IF NOT EXISTS ask_quantity INTEGER CHECK (ask_quantity > 0);
