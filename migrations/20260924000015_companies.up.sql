-- v62: companhias (guildas de jogador). Membro em uma só companhia; a
-- companhia sem ninguém sai do banco.
CREATE TABLE companies (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    tag TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE company_members (
    character_id UUID PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    company_id UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    -- Nome à mostra quando entrou (membro offline também aparece na lista).
    name TEXT NOT NULL,
    leader BOOLEAN NOT NULL DEFAULT false,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_company_members_company ON company_members (company_id);
