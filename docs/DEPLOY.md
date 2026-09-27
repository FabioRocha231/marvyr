# Deploy e release do Marvyr

Guia operacional do alpha público: servidor no Dokploy, cliente Windows via
GitHub Actions + itch.io.

## Topologia (Dokploy)

Um projeto Dokploy **Marvyr** com três serviços na mesma rede interna:

| Serviço | Origem | Exposição |
|---|---|---|
| `marvyr-server` | Application, `Dockerfile.server` (raiz do repo) | UDP `5000` publicado direto no host (**fora do Traefik**) |
| `marvyr-auth` | Application, `Dockerfile.auth` (raiz do repo) | HTTPS via Traefik: `auth.marvyr.game` → container `8080` |
| `marvyr-db` | Database PostgreSQL | só rede interna; **nunca** publicar `5432` |

```
jogador ──UDP 5000──────────────▶ marvyr-server ─┐
jogador ──HTTPS 443─▶ Traefik ──▶ marvyr-auth ───┼──▶ marvyr-db (rede interna)
                                                 │
                     MARVYR_JWT_SECRET compartilhado
```

### Porta UDP do servidor

O Traefik do Dokploy só roteia HTTP/TCP, então o jogo **não** passa por ele.
Em `marvyr-server` → *Advanced* → *Ports*: publicada `5000`, alvo `5000`,
protocolo **UDP** (modo de publicação `host` se disponível, para o IP do
cliente chegar intacto). Não configure domínio para esse app.

### DNS

- `A play.marvyr.game` → IP público da VPS (endereço que os clientes usam, `play.marvyr.game:5000`).
- `A auth.marvyr.game` → mesmo IP (Traefik emite o certificado Let's Encrypt).

### Firewall da VPS

| Porta | Abrir? |
|---|---|
| `5000/udp` | sim (jogo) |
| `80/tcp`, `443/tcp` | sim (Traefik/ACME) |
| `22/tcp` | sim, de preferência restrito ao seu IP |
| `3000/tcp` (painel Dokploy) | só se necessário; melhor via domínio HTTPS |
| `5432/tcp` | **nunca** |

Atenção: o Docker escreve regras de iptables que ignoram o `ufw`. Por isso a
única defesa real para o Postgres é **não publicar a porta**.

## Variáveis de ambiente

### `marvyr-server`

| Variável | Valor em produção | Notas |
|---|---|---|
| `MARVYR_PORT` | `5000` | porta UDP (padrão 5000) |
| `MARVYR_ENV` | `production` | `development` só local |
| `MARVYR_DATABASE_URL` | `postgres://marvyr:<senha>@marvyr-db:5432/marvyr` | fail-closed: se definida e o banco não abrir, o servidor sai |
| `MARVYR_JWT_SECRET` | 64 caracteres hex | ≥ 32 bytes, **igual** ao do auth; obrigatório em produção |
| `MARVYR_ALLOW_ANON` | *(não definir)* | `1` aceita identidade anônima — só dev, **nunca** em produção |
| `MARVYR_MAX_CLIENTS` | `64` | padrão 64 |
| `RUST_LOG` | `info,marvyr_server=info` | |
| `MARVYR_REPORT_DIR` | `/data/reports` | `session-summary.json` é gravado aqui no desligamento |
| `MARVYR_WORLD_SEED` | *(não definir)* ou um número | seed do mundo procedural; sem ela, a seed padrão do código. `0` = mapa clássico feito à mão. **Trocar a seed é trocar o mundo**: faça wipe junto (navios atracados voltam ao próprio porto, mas carga e rotas perdem o sentido) |
| `MARVYR_SEA_EVENT` | *(não definir)* | `tempest`/`fleet`/`kraken`/`tide` força um evento de mar no boot — só teste |
| `MARVYR_WEB_PORT` | `5001` ou *(não definir)* | liga o WebTransport para o jogo no navegador (ver "Build web") |

Monte um volume persistente em `/data` (relatórios de sessão).

### `marvyr-auth`

| Variável | Valor em produção | Notas |
|---|---|---|
| `MARVYR_AUTH_ADDR` | `0.0.0.0:8080` | padrão |
| `MARVYR_JWT_SECRET` | mesmo do servidor | |
| `MARVYR_DATABASE_URL` | mesma string do servidor | obrigatório; as migrations rodam no boot |
| `MARVYR_TOKEN_TTL_SECS` | `604800` | validade do token (padrão 7 dias) |
| `MARVYR_TRUST_PROXY` | `1` atrás do Traefik | usa a entrada mais à direita do `X-Forwarded-For` (a que o Traefik anexa) no limite de tentativas; supõe **um** proxy na frente |
| `RUST_LOG` | `info` | |

### Cliente (`Marvyr.exe`)

Prioridade, da mais forte para a mais fraca:

1. CLI: `--server host:porta`, `--auth-url URL`
2. Ambiente: `MARVYR_SERVER_HOST` + `MARVYR_PORT`, `MARVYR_AUTH_URL`
3. Arquivo `marvyr.toml` ao lado do exe: `server = "host:porta"`, `auth_url = "https://..."`
4. Padrões gravados no build: `MARVYR_DEFAULT_SERVER`, `MARVYR_DEFAULT_AUTH_URL`
5. `localhost` — **só** em build de dev

Variáveis de **build** (lidas pelo `cargo build`, não em runtime):

| Variável | Uso |
|---|---|
| `MARVYR_PUBLIC_BUILD=1` | build público: sem fallback para localhost (mostra "Servidor do Marvyr não configurado") |
| `MARVYR_DEFAULT_SERVER` / `MARVYR_DEFAULT_AUTH_URL` | endereços padrão gravados no exe |
| `MARVYR_BUILD_SHA` / `MARVYR_VERSION_LABEL` | metadados exibidos pelo jogo |

## Build web (navegador)

O browser não abre UDP: o jogo web entra por **WebTransport** (QUIC, que
também é UDP, numa porta separada). O lightyear 0.19 só aceita certificado
fixado pelo hash, e o browser só fixa certificado de **até 14 dias**. Então:

1. `marvyr-server` com `MARVYR_WEB_PORT=5001` gera um certificado
   autoassinado a cada boot e grava o hash na tabela `web_cert`.
2. `marvyr-auth` entrega o hash em `GET /v1/web-cert` (HTTPS do Traefik, o
   que torna a fixação confiável; CORS aberto, a API não usa cookie).
3. O client web busca o hash e conecta em `https://<IPv4>:5001`.

Operação:

- Publique `5001/udp` em host mode, como a `5000` (firewall também).
- **Reinicie o servidor antes de 13 dias de uptime**: depois disso o
  certificado vence, `/v1/web-cert` responde 404 e browser novo não entra
  (o nativo não é afetado). Um deploy semanal resolve.
- Variável do repositório `MARVYR_WEB_SERVER` = `IPv4:5001` (IP, não
  hostname: o wasm não resolve DNS). Com ela, o release gera
  `Marvyr-v*-web.zip` e o job `itch-web` publica no canal `html5`. Na
  primeira vez, marque esse upload como "jogável no navegador" no itch.

No navegador, sessão e preferências não são salvas (o `std::fs` não existe
lá): o jogador faz login a cada visita.

Dev local, sem `marvyr-auth`:

```sh
MARVYR_PORT=5094 MARVYR_WEB_PORT=5001 MARVYR_ENV=development MARVYR_ALLOW_ANON=1 \
  cargo run -p marvyr-server        # o log mostra `digest=<hex>`
cargo install trunk && cd crates/client && trunk serve
# abra http://127.0.0.1:8080/?cert=<hex>  (só build de dev aceita ?cert=)
```

Dados do jogador: `%APPDATA%\Marvyr` (Windows), `~/Library/Application Support/Marvyr` (macOS), `~/.local/share/Marvyr` (Linux).

## Segredo JWT

```sh
openssl rand -hex 32
```

Cole o mesmo valor em `MARVYR_JWT_SECRET` de `marvyr-server` e `marvyr-auth`
(Environment do Dokploy, nunca no repositório). Trocar o segredo invalida todas
as sessões: reinicie os dois serviços juntos.

## PostgreSQL

- Serviço Database do Dokploy, sem porta externa. Os apps conectam por `marvyr-db:5432`.
- As migrations rodam sozinhas no boot do servidor (embutidas via `sqlx::migrate!`).
- **Backups**: em *Backups* do banco, agende um backup diário (ex.: `0 4 * * *`)
  para um destino S3 (*Settings → S3 Destinations*) com retenção de pelo menos
  7 diários. Sem S3, faça `pg_dump` diário por cron para fora da VPS.
- Teste a restauração ao menos uma vez antes de abrir o alpha.

### Cosméticos (concessão pela administração)

Velas e bandeiras são **só aparência** (nunca stats). Não há loja: a
administração concede pelo nome de login do capitão, dentro do contêiner do
servidor. O capitão vê o item no próximo login e escolhe na aba *Equipamento*
do porto. Revogar também vale a partir do próximo login do capitão.

```sh
C=$(docker ps -qf name=marvyr-server)
docker exec $C marvyr-db-migrate list-cosmetics            # catálogo
docker exec $C sh -c 'marvyr-db-migrate --database-url "$MARVYR_DATABASE_URL" \
  grant-cosmetic --captain NOME --cosmetic sail-gold --by admin'
docker exec $C sh -c 'marvyr-db-migrate --database-url "$MARVYR_DATABASE_URL" \
  list-cosmetics --captain NOME'
docker exec $C sh -c 'marvyr-db-migrate --database-url "$MARVYR_DATABASE_URL" \
  revoke-cosmetic --captain NOME --cosmetic sail-gold'
```

Em dev sem banco, `MARVYR_DEV_COSMETICS=1` libera o catálogo inteiro (ignorado
com `MARVYR_ENV=production`).

### Telemetria de retenção

O servidor grava em `captain_events` (lote a cada 30 s) o que cada capitão
faz: `sessao` (entrou; detalhe = casco), `renome` (usou a mecânica, uma vez
por dia por motivo), `meta` (meta do Diário cumprida) e `livro` (entrada nova
do Livro). O jogo nunca lê essa tabela. Consultas (psql ou Grafana com o
Postgres como fonte):

```sql
-- Retenção D1/D7: capitães que voltaram 1 e 7 dias depois da 1ª sessão.
WITH first AS (SELECT character_id, min(at)::date AS d0
               FROM captain_events WHERE kind = 'sessao' GROUP BY 1)
SELECT d0, count(*) AS novos,
  count(*) FILTER (WHERE EXISTS (SELECT 1 FROM captain_events e
    WHERE e.character_id = f.character_id AND e.kind = 'sessao'
    AND e.at::date = f.d0 + 1)) AS d1,
  count(*) FILTER (WHERE EXISTS (SELECT 1 FROM captain_events e
    WHERE e.character_id = f.character_id AND e.kind = 'sessao'
    AND e.at::date = f.d0 + 7)) AS d7
FROM first f GROUP BY d0 ORDER BY d0 DESC;

-- Mecânicas mais usadas (capitães distintos por dia).
SELECT at::date, detail, count(DISTINCT character_id)
FROM captain_events WHERE kind = 'renome' GROUP BY 1, 2 ORDER BY 1 DESC, 3 DESC;

-- Metas do Diário mais e menos cumpridas nos últimos 7 dias.
SELECT detail, count(*) FROM captain_events
WHERE kind = 'meta' AND at > now() - interval '7 days' GROUP BY 1 ORDER BY 2 DESC;
```

## Operação

### Reinício ordenado

O servidor persiste o estado ao receber `SIGTERM` (`STOPSIGNAL SIGTERM` no
Dockerfile). O Docker espera 10 s por padrão antes do `SIGKILL`; aumente o
grace period do serviço (Swarm `StopGracePeriod`, ex.: 30 s) se o desligamento
passar disso.

1. Avise os jogadores (reinício em N minutos).
2. Dokploy → `marvyr-server` → *Stop* (ou *Deploy* para uma nova versão).
3. Confira no log que o estado foi salvo e que `session-summary.json` foi escrito em `/data/reports`.
4. *Start*/aguarde o deploy e confirme a linha de escuta na porta 5000.

Nunca use `docker kill` / `SIGKILL` em produção: perde o que não foi persistido.

### Logs

- Dokploy → app → aba *Logs* (tempo real).
- Na VPS: `docker service logs -f --tail 200 <serviço>` (Swarm) ou `docker logs -f --tail 200 <container>`.
- Mais detalhe temporário: `RUST_LOG=debug,marvyr_server=debug` e redeploy (volte ao normal depois).

## Release do cliente

1. Garanta que `main` está verde no CI.
2. Configure uma vez no GitHub (*Settings → Secrets and variables → Actions*):
   - variáveis: `MARVYR_DEFAULT_SERVER` (`play.marvyr.game:5000`),
     `MARVYR_DEFAULT_AUTH_URL` (`https://auth.marvyr.game`),
     `ITCH_TARGET` (`usuario/marvyr`; vazio desativa o itch);
   - secret: `BUTLER_API_KEY` (`butler login` → chave em itch.io → *API keys*);
   - environment `itch` com *required reviewers* para aprovar o envio.
3. Crie e envie a tag:
   ```sh
   git tag v0.1.0-alpha.1
   git push origin v0.1.0-alpha.1
   ```
4. O workflow `Release` roda `checks` → `package` em matriz (executa
   `scripts/package.sh` em Windows, Linux e macOS e publica os artefatos
   `Marvyr-windows`, `Marvyr-linux` e `Marvyr-macos`) → `publish` (um único
   GitHub Release com os três arquivos) e `itch` (após aprovação, `butler push`
   de cada plataforma nos canais `windows`, `linux` e `mac`).
5. Sem tag: *Actions → Release → Run workflow* com `version` gera só os artefatos.

| Plataforma | Runner | Arquivo |
|---|---|---|
| Windows | `windows-latest` | `Marvyr-v<versão>-windows-x86_64.zip` (`Marvyr.exe` + `assets/`) |
| Linux | `ubuntu-22.04` (glibc antiga, roda em mais distros) | `Marvyr-v<versão>-linux-x86_64.tar.gz` (`Marvyr` + `assets/`) |
| macOS | `macos-latest` | `Marvyr-v<versão>-macos-universal.zip` (`Marvyr.app`, Apple Silicon + Intel) |

O `.app` do macOS tem assinatura ad-hoc, sem notarização: na primeira abertura
o jogador usa *botão direito → Abrir* (o `README.txt` explica). Notarizar exige
conta de desenvolvedor da Apple.

Empacotar localmente: `bash scripts/package.sh <windows|linux|macos>` (Windows
com Git Bash; o macOS precisa do alvo `x86_64-apple-darwin` para o universal).
O script é a fonte única do conteúdo do pacote: executável, `assets/`
(`marvyr`, `external`, `shaders`; `dev` fica fora), `LICENSE`,
`ATTRIBUTION.md`, `VERSION` e `README.txt`. O cliente procura `assets/` e
`marvyr.toml` ao lado do executável; no macOS os assets ficam em `Marvyr.app/Contents/Resources` (dentro do selo da assinatura).

Ferramenta de playtest em dev (não distribuída): `cargo run --bin marvyr_playtest --release`.

## Testes de aceitação manuais

Antes de anunciar uma versão:

- [ ] **Local**: servidor + cliente na mesma máquina conectam, criam conta e jogam.
- [ ] **LAN**: cliente em outra máquina da rede conecta via `--server <ip-lan>:5000`.
- [ ] **Internet**: cliente fora da rede conecta em `play.marvyr.game:5000` com o build público, sem argumentos.
- [ ] **3 clientes** simultâneos se veem e interagem sem desync visível.
- [ ] **Persistência no reinício**: jogar, fazer *Stop/Start* do servidor, reconectar e encontrar navio/inventário/ouro como antes.
- [ ] **Máquina limpa**: em Windows, Linux e macOS sem Rust instalado, extrair o pacote, abrir o jogo, criar conta e jogar.
- [ ] **Versão incompatível**: cliente com `PROTOCOL_VERSION` diferente recebe mensagem clara, sem crash.
- [ ] **Servidor offline**: com o servidor parado, o cliente mostra erro de conexão legível e não trava.
