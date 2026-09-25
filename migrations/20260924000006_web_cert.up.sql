-- Build web: hash SHA-256 (hex) do certificado WebTransport do game server.
-- Autoassinado e renovado a cada boot (o browser só fixa certificado de até
-- 14 dias); o marvyr-auth entrega ao browser em GET /v1/web-cert. Linha única.
CREATE TABLE web_cert (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    digest TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
