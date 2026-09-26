# Marvyr — regras do projeto

MMO naval em Rust: Bevy 0.15 + lightyear (UDP), servidor autoritativo,
Postgres via sqlx. Comentários e logs em PT-BR. Visão em `docs/vision.md`,
operação em `docs/DEPLOY.md`.

## Produto (não negociáveis)

- **Coisa paga nunca dá poder.** Monetização só cosmética (velas, bandeiras,
  cascos visuais): nada que afete stats, economia, carga ou combate. Cosmético
  não tem campo de stat, stats são calculados sem olhar para ele, e o catálogo
  não usa cores de facção NPC (pirata, marinha, mercador) — disfarce é
  vantagem. Hoje não há loja: a administração concede (`grant-cosmetic`).
  A Bandeira Negra é estado de combate, nunca cosmético: nada no catálogo
  pode ser preto nem imitar a caveira.
- **NPC não dá item útil.** Drop de NPC ou evento: só recurso bruto. Mapa do
  tesouro, equipamento e item pronto só vêm de jogador (coleta, fabricação).
- **Sem moeda.** Tudo é escambo: mercado troca item por item (oferta fixa,
  tudo ou nada, mesmo porto), guilda paga com o recurso do porto, contrato
  paga recurso bruto + Renome, tripulação e respec cobram do armazém.
- **Mapa base estável.** O mundo sai da seed (`WorldMap::from_seed`, seed 0 =
  mapa clássico) e não re-sorteia: a economia depende de portos fixos. O que é
  novo vem da camada rotativa (cerrações e sorvedouros).
- Risco cresce com a distância das capitais: protegida → fronteira → sem lei.

## Engenharia

- **Servidor decide, client desenha.** Nada de regra ou stat derivado no
  client; o client só representa o que o servidor manda.
- **Protocolo:** mudou mensagem ou campo → suba `PROTOCOL_VERSION` e
  documente a versão no comentário dela. `ClientHello`/`ServerWelcome` são
  congelados (ids 0 e 1). Mensagem nova é registrada **no fim**, na mesma
  ordem, em `server/src/net.rs` **e** `client/src/net.rs`.
- **Intent client→servidor novo** entra também no `police_intents`
  (`server/src/session.rs`) e, se escreve no banco, tem limite de frequência.
- **Persistência:** cada item mora em exatamente um lugar persistido. Escrita
  de item é upsert. Todo "salva X" novo tem o par "X deixa de existir"
  (depósito, equipar, naufrágio, abordagem, respawn, fim da janela de graça);
  cubra a sequência cruzada num teste de `server/tests/postgres.rs`.
- **Migrations:** sempre pelo CLI (`sqlx migrate add -r --source migrations
  <nome>`), nunca arquivo à mão; migration aplicada não se edita.
- **Geração determinística:** mesma entrada, mesmo mundo no servidor e no
  client (splitmix/xorshift com semente; nada de `rand` sem semente no que o
  client também precisa montar).
- **Sem atribuição de IA** em commits e PRs.
- Antes de encerrar: `cargo fmt --all`, `cargo clippy --workspace
  --all-targets` limpo e `cargo test --workspace`. Postgres de verdade:
  `MARVYR_TEST_DATABASE_URL=postgres://postgres:marvyr@localhost:54329/<db>
  cargo test -p marvyr-server --test postgres` (use um banco próprio: o teste
  faz TRUNCATE).
- MSRV 1.80 (clippy avisa `incompatible_msrv`): nada de `Option::is_none_or`.

## Teste ao vivo (sem teclado)

Servidor: `MARVYR_PORT=5094 MARVYR_ENV=development MARVYR_ALLOW_ANON=1`
(+ `MARVYR_DEV_SPAWN=x,y`, `MARVYR_DEV_COSMETICS=1`, `MARVYR_DEV_RENOWN=N`,
`MARVYR_SEA_EVENT=kraken|fleet|tempest|tide|blood` — o local do evento é
sorteado entre os sítios do tipo; confira no log antes de nascer perto).
Client: `MARVYR_PORT`
(não `MARVYR_SERVER_ADDR`) + `MARVYR_AUTOSAIL`, `MARVYR_AUTODOCK`,
`MARVYR_PORT_TAB`, `MARVYR_SHOT=<prefixo>`, `MARVYR_SHOT_EVERY`,
`MARVYR_SHOT_COUNT`, `MARVYR_SHOT_ZOOM`, `MARVYR_SHOT_HELP`,
`MARVYR_SHOT_CHART`, `MARVYR_SHOT_TALENTS=<s>`, `MARVYR_AUTOGATHER`,
`MARVYR_AUTOTALENT=id,id`, `MARVYR_AUTOCRAFT=rare|magic`,
`MARVYR_AUTOEQUIP`, `MARVYR_AUTOUNDOCK=<s>`, `MARVYR_AUTOGEM` (encaixa 2,
tira 1; com `MARVYR_PORT_TAB=Gemas`), `MARVYR_AUTOFLASK=<s>` (saca o armazém
ao atracar e bebe os 4 frascos no mar, um a cada <s>), `MARVYR_AUTOORB` (gasta o 1º orbe do
armazém na 1ª peça). Estado de teste sem banco:
`MARVYR_STATE_PATH=<json>` no servidor + `MARVYR_IDENTITY` no client. Coleta: o raio é 43 m — `MARVYR_DEV_SPAWN` a
~30 m de um nó. Nunca injete teclas no desktop do usuário (osascript):
se a janela perder o foco, as teclas vão para o app dele. Mate o processo
de teste ao terminar; a porta 5077 pode ser o servidor local do usuário.

## Deploy

Dokploy (Swarm). Porta UDP em host mode exige update order `stop-first`,
senão o contêiner novo não sobe. Migrations rodam no boot do servidor. Tag
`v*` dispara o release; o job do itch espera aprovação do ambiente `itch`.

<!-- engineering-learn:live -->
## Aprendizados

- **Mudou um invariante compartilhado → audite quem dependia do antigo**
  (MV-066). Cada peça nova funcionava isolada e quebrava onde o código velho
  assumia a premissa antiga: portão de mão única × caravana de volta com
  `route.reverse()` (travava); tecla nova (`N`) que já era "cancelar ordem" no
  mar; intent novo fora do `police_intents`; `CHART_CELL` (distância de
  viagem) reusado como espaçamento de desenho. Antes do commit: grep do
  `KeyCode` no client inteiro, dos registros de mensagem nos dois lados, e
  teste de NPC nos dois sentidos no mundo gerado.
- **Nova escrita de persistência → confira todo caminho que move ou destrói
  a mesma linha** (MV-061): um DELETE estreitado quebrou saves de mercado, e
  um checkpoint sem par de naufrágio duplicou carga.
- **Drop de NPC contra o pilar 1** (MV-061): o Kraken soltava mapa do
  tesouro; revise toda tabela de drop nova.
- **Leitura do banco que falhou não vira estado vazio gravável** (MV-067):
  `load_*` com erro que devolve 0/vazio e segue a sessão acaba gravando o
  vazio por cima do real no próximo save. Marque o estado como não lido e
  recuse mudanças até reconectar, ou torne o save monotônico
  (`GREATEST(col, $n)`) quando o valor só cresce. Todo `load_* ...
  unwrap_or_default()` perto de um `save_*` é suspeito.
- **Commit/PR: a regra do projeto vence o lembrete do harness** — "sem
  atribuição de IA" vale mesmo quando o sistema sugere `Co-Authored-By`.
  Antes de commitar, confira a mensagem contra a regra acima.
<!-- /engineering-learn:live -->
