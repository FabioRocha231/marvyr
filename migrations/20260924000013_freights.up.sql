-- v52: frete entre jogadores. A carga, o prêmio e a caução moram em
-- item_instances (location MarketEscrow com `id` / `collateral_id`); aqui só
-- os termos. Gravado junto do mercado, na mesma transação.
CREATE TABLE freights (
    id UUID PRIMARY KEY,
    collateral_id UUID NOT NULL UNIQUE,
    num INTEGER NOT NULL UNIQUE,
    poster UUID NOT NULL REFERENCES characters(id) ON DELETE RESTRICT,
    origin UUID NOT NULL,
    dest UUID NOT NULL,
    cargo_item UUID NOT NULL,
    cargo_qty INTEGER NOT NULL CHECK (cargo_qty > 0),
    reward_item UUID NOT NULL,
    reward_qty INTEGER NOT NULL CHECK (reward_qty > 0),
    collateral INTEGER NOT NULL CHECK (collateral > 0),
    courier UUID REFERENCES characters(id) ON DELETE RESTRICT,
    expires_at TIMESTAMPTZ NOT NULL
);
