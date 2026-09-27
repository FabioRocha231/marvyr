//! Sessão do Public Alpha (MV-061): quem pode entrar e quem é expulso.
//!
//! - Identidade: o `ClientHello.identity` é o JWT do `marvyr-auth`. Token
//!   anônimo (UUID de dev) só vale fora de produção.
//! - Conexões recusadas recebem `ServerWelcome` com motivo e são
//!   desconectadas logo depois (a mensagem precisa sair antes do kick).
//! - Orçamento de intents: client que inunda o servidor de mensagens é
//!   desconectado — hardening mínimo, não anti-cheat.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_protocol::MAX_IDENTITY_LEN;
pub use marvyr_protocol::{REASON_BAD_SESSION, REASON_NO_SESSION};
use tracing::{info, warn};

/// Configuração de sessão lida do ambiente no boot.
#[derive(Resource, Clone)]
pub struct AuthConfig {
    pub production: bool,
    /// Segredo compartilhado com o `marvyr-auth` (`MARVYR_JWT_SECRET`).
    pub jwt_secret: Option<Vec<u8>>,
    /// Aceita token anônimo (dev/testes). Nunca em produção.
    pub allow_anon: bool,
    pub max_clients: usize,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            production: false,
            jwt_secret: None,
            allow_anon: true,
            max_clients: 64,
        }
    }
}

impl AuthConfig {
    /// Lê `MARVYR_ENV`, `MARVYR_JWT_SECRET`, `MARVYR_ALLOW_ANON` e
    /// `MARVYR_MAX_CLIENTS`. Produção sem segredo válido não sobe
    /// (fail-closed: servidor público sem login seria aberto a qualquer um).
    pub fn from_env() -> Self {
        let production = std::env::var("MARVYR_ENV").is_ok_and(|env| env == "production");
        let jwt_secret = std::env::var("MARVYR_JWT_SECRET")
            .ok()
            .filter(|secret| !secret.is_empty())
            .map(String::into_bytes);
        if let Some(secret) = &jwt_secret {
            assert!(
                secret.len() >= marvyr_auth_token::MIN_SECRET_LEN,
                "MARVYR_JWT_SECRET precisa de pelo menos {} bytes",
                marvyr_auth_token::MIN_SECRET_LEN
            );
        }
        if production {
            assert!(
                jwt_secret.is_some(),
                "MARVYR_ENV=production exige MARVYR_JWT_SECRET (login obrigatório)"
            );
        }
        let anon_requested = std::env::var("MARVYR_ALLOW_ANON").map(|value| value == "1");
        let allow_anon = match anon_requested {
            Ok(requested) => requested && !production,
            // Sem opinião explícita: dev aceita anônimo só se não há login.
            Err(_) => !production && jwt_secret.is_none(),
        };
        if production && anon_requested == Ok(true) {
            warn!("MARVYR_ALLOW_ANON ignorado em produção");
        }
        let max_clients = std::env::var("MARVYR_MAX_CLIENTS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|max| *max > 0)
            .unwrap_or(64);
        Self {
            production,
            jwt_secret,
            allow_anon,
            max_clients,
        }
    }
}

/// Identidade resolvida de um hello aceito.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIdentity {
    /// Chave persistente do personagem (`account:<uuid>` ou token de dev).
    pub key: String,
    /// Nome exibido (capitão) — o token anônimo não tem nome.
    pub display_name: String,
}

pub const REASON_LOGIN_REQUIRED: &str = "Este servidor exige login com uma conta Marvyr.";
pub const REASON_FULL: &str = "Servidor cheio. Tente de novo em alguns minutos.";
pub const REASON_ALREADY_AT_SEA: &str =
    "Seu capitão já está conectado em outra sessão. Feche o outro jogo e tente de novo.";
pub const REASON_FLOOD: &str = "Desconectado por excesso de mensagens.";

/// Valida o token do hello. Pura: testável sem rede.
pub fn resolve_identity(
    identity: &str,
    config: &AuthConfig,
) -> Result<ResolvedIdentity, &'static str> {
    let token = identity.trim();
    if token.is_empty() {
        return Err(REASON_NO_SESSION);
    }
    if token.len() > MAX_IDENTITY_LEN {
        return Err(REASON_BAD_SESSION);
    }
    if marvyr_auth_token::looks_like_jwt(token) {
        let Some(secret) = &config.jwt_secret else {
            return Err(REASON_BAD_SESSION);
        };
        let claims = marvyr_auth_token::verify(token, secret).map_err(|_| REASON_BAD_SESSION)?;
        return Ok(ResolvedIdentity {
            key: crate::market::account_identity(claims.sub),
            display_name: claims.username,
        });
    }
    if !config.allow_anon {
        return Err(REASON_LOGIN_REQUIRED);
    }
    // Chave `account:<uuid>` só nasce de JWT verificado.
    if token.starts_with(crate::market::ACCOUNT_IDENTITY_PREFIX) {
        return Err(REASON_BAD_SESSION);
    }
    Ok(ResolvedIdentity {
        key: token.to_owned(),
        display_name: String::from("Capitão"),
    })
}

/// Clients recusados aguardando o kick (o `ServerWelcome` sai no fim do
/// frame; desconectar no mesmo tick engoliria a mensagem).
#[derive(Resource, Default)]
pub struct PendingKicks(pub Vec<(ClientId, f32)>);

/// Atraso entre a recusa e o kick.
const KICK_DELAY_SECS: f32 = 0.5;

impl PendingKicks {
    pub fn schedule(&mut self, client_id: ClientId, now: f32) {
        if !self.0.iter().any(|(pending, _)| *pending == client_id) {
            self.0.push((client_id, now + KICK_DELAY_SECS));
        }
    }
}

pub fn kick_rejected(mut commands: Commands, time: Res<Time>, mut kicks: ResMut<PendingKicks>) {
    let now = time.elapsed_secs();
    kicks.0.retain(|(client_id, at)| {
        if *at > now {
            return true;
        }
        info!(client = ?client_id, "client recusado desconectado");
        commands.disconnect(*client_id);
        false
    });
}

/// Mensagens confiáveis por segundo por client antes do kick. Um jogador
/// apertando teclas manda poucas por segundo; UI automatizada de dev fica
/// bem abaixo disso.
const RELIABLE_BUDGET_PER_SEC: u32 = 60;
/// `ShipInput` vai por frame (até ~144 Hz com monitor rápido).
const INPUT_BUDGET_PER_SEC: u32 = 300;

/// Intent que grava no banco (serviço de porto): no máximo um a cada
/// `secs` por client. `true` = pode seguir (e marca a hora).
pub fn intent_ready(
    last: &mut HashMap<ClientId, f64>,
    client_id: ClientId,
    now: f64,
    secs: f64,
) -> bool {
    if last.get(&client_id).is_some_and(|at| now - at < secs) {
        return false;
    }
    last.insert(client_id, now);
    true
}

/// Contagem de mensagens por client na janela de 1s corrente.
#[derive(Resource, Default)]
pub struct IntentBudget {
    window_start: f32,
    reliable: HashMap<ClientId, u32>,
    input: HashMap<ClientId, u32>,
}

impl IntentBudget {
    /// Registra `reliable`/`input` mensagens; devolve os clients que
    /// estouraram o orçamento nesta janela (cada um uma única vez).
    fn charge(
        &mut self,
        now: f32,
        reliable: impl IntoIterator<Item = ClientId>,
        input: impl IntoIterator<Item = ClientId>,
    ) -> Vec<ClientId> {
        if now - self.window_start >= 1.0 {
            self.window_start = now;
            self.reliable.clear();
            self.input.clear();
        }
        let mut flooders = Vec::new();
        for client in reliable {
            let count = self.reliable.entry(client).or_default();
            *count += 1;
            if *count == RELIABLE_BUDGET_PER_SEC + 1 {
                flooders.push(client);
            }
        }
        for client in input {
            let count = self.input.entry(client).or_default();
            *count += 1;
            if *count == INPUT_BUDGET_PER_SEC + 1 {
                flooders.push(client);
            }
        }
        flooders
    }
}

type Rx<'w, 's, M> = EventReader<'w, 's, ServerReceiveMessage<M>>;

/// Lê TODAS as intents de client→servidor (cada `EventReader` tem cursor
/// próprio: os handlers continuam recebendo tudo) e expulsa quem inunda.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn police_intents(
    time: Res<Time>,
    mut budget: ResMut<IntentBudget>,
    mut kicks: ResMut<PendingKicks>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut input: Rx<marvyr_protocol::ShipInput>,
    mut ship: (
        Rx<marvyr_protocol::ClientHello>,
        Rx<marvyr_protocol::Dock>,
        Rx<marvyr_protocol::Undock>,
        Rx<marvyr_protocol::EquipItem>,
        Rx<marvyr_protocol::UnequipItem>,
        Rx<marvyr_protocol::SetBlackFlag>,
        Rx<marvyr_protocol::SelectAmmo>,
        Rx<marvyr_protocol::LootWreck>,
    ),
    mut economy: (
        Rx<marvyr_protocol::GatherNode>,
        Rx<marvyr_protocol::CraftItem>,
        Rx<marvyr_protocol::StorageDepositAll>,
        Rx<marvyr_protocol::StorageWithdrawAll>,
        Rx<marvyr_protocol::CreateSellOrder>,
        Rx<marvyr_protocol::CancelSellOrder>,
        Rx<marvyr_protocol::BuySellOrder>,
        Rx<marvyr_protocol::SellToGuild>,
    ),
    mut sea: (
        Rx<marvyr_protocol::AcceptContract>,
        Rx<marvyr_protocol::AbandonContract>,
        Rx<marvyr_protocol::SetRepair>,
        Rx<marvyr_protocol::BoardShip>,
        Rx<marvyr_protocol::HireCrew>,
        Rx<marvyr_protocol::DigTreasure>,
        Rx<marvyr_protocol::OnboardingProgress>,
        Rx<marvyr_protocol::WearCosmetic>,
    ),
    mut captain: (
        Rx<marvyr_protocol::AllocateTalent>,
        Rx<marvyr_protocol::RespecTalents>,
        Rx<marvyr_protocol::LockTarget>,
        Rx<marvyr_protocol::SocketGem>,
        Rx<marvyr_protocol::UnsocketGem>,
        Rx<marvyr_protocol::UseFlask>,
        Rx<marvyr_protocol::StorageDeposit>,
        Rx<marvyr_protocol::StorageWithdraw>,
        Rx<marvyr_protocol::ApplyOrb>,
    ),
    mut extra: (
        Rx<marvyr_protocol::CastLine>,
        Rx<marvyr_protocol::RaiseLighthouse>,
        Rx<marvyr_protocol::ThrowBottle>,
        Rx<marvyr_protocol::PickBottle>,
        Rx<marvyr_protocol::PostFreight>,
        Rx<marvyr_protocol::AcceptFreight>,
        Rx<marvyr_protocol::CancelFreight>,
    ),
) {
    let mut reliable: Vec<ClientId> = Vec::new();
    macro_rules! drain {
        ($($reader:expr),*) => {
            $(reliable.extend($reader.read().map(|event| event.from()));)*
        };
    }
    drain!(ship.0, ship.1, ship.2, ship.3, ship.4, ship.5, ship.6, ship.7);
    drain!(economy.0, economy.1, economy.2, economy.3, economy.4, economy.5, economy.6, economy.7);
    drain!(sea.0, sea.1, sea.2, sea.3, sea.4, sea.5, sea.6, sea.7);
    drain!(
        captain.0, captain.1, captain.2, captain.3, captain.4, captain.5, captain.6, captain.7,
        captain.8
    );
    drain!(extra.0, extra.1, extra.2, extra.3, extra.4, extra.5, extra.6);
    let inputs: Vec<ClientId> = input.read().map(|event| event.from()).collect();
    let now = time.elapsed_secs();
    for client_id in budget.charge(now, reliable, inputs) {
        warn!(client = ?client_id, "client inundando o servidor; desconectando");
        let _ = connection_manager.send_message::<crate::net::ReliableChannel, _>(
            client_id,
            &marvyr_protocol::WorldEvent {
                text: String::from(REASON_FLOOD),
                kind: marvyr_protocol::WorldEventKind::Alert,
            },
        );
        kicks.schedule(client_id, now);
    }
}

/// Esconde a senha de uma URL de banco para log (`postgres://u:***@host/db`).
/// A query string sai inteira: `?password=`/`sslpassword=` também são segredo.
pub fn redact_url(url: &str) -> String {
    let url = url.split('?').next().unwrap_or_default();
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::from("***");
    };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    match authority.rsplit_once('@') {
        Some((credentials, host)) => {
            let user = credentials.split(':').next().unwrap_or_default();
            format!("{scheme}://{user}:***@{host}{path}")
        }
        None => format!("{scheme}://{authority}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn production() -> AuthConfig {
        AuthConfig {
            production: true,
            jwt_secret: Some(SECRET.to_vec()),
            allow_anon: false,
            max_clients: 4,
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn valid_jwt_resolves_to_account_key_never_the_token() {
        let account = uuid::Uuid::new_v4();
        let token = marvyr_auth_token::issue(account, "barba_ruiva", SECRET, now(), 60).unwrap();
        let resolved = resolve_identity(&token, &production()).unwrap();
        assert_eq!(resolved.key, crate::market::account_identity(account));
        assert_eq!(resolved.display_name, "barba_ruiva");
        assert_eq!(
            crate::market::account_of_identity(&resolved.key),
            Some(account)
        );
    }

    #[test]
    fn production_refuses_anonymous_empty_oversized_and_forged_tokens() {
        let config = production();
        assert_eq!(resolve_identity("  ", &config), Err(REASON_NO_SESSION));
        assert_eq!(
            resolve_identity("uuid-de-dev", &config),
            Err(REASON_LOGIN_REQUIRED)
        );
        let huge = "a".repeat(MAX_IDENTITY_LEN + 1);
        assert_eq!(resolve_identity(&huge, &config), Err(REASON_BAD_SESSION));
        let forged = marvyr_auth_token::issue(
            uuid::Uuid::new_v4(),
            "x",
            b"ffffffffffffffffffffffffffffffff",
            now(),
            60,
        )
        .unwrap();
        assert_eq!(resolve_identity(&forged, &config), Err(REASON_BAD_SESSION));
    }

    #[test]
    fn dev_accepts_anonymous_token() {
        let resolved = resolve_identity("dev-token", &AuthConfig::default()).unwrap();
        assert_eq!(resolved.key, "dev-token");
        let forged = format!("account:{}", uuid::Uuid::new_v4());
        assert!(
            resolve_identity(&forged, &AuthConfig::default()).is_err(),
            "token anônimo não se passa por conta"
        );
    }

    #[test]
    fn budget_flags_a_flooder_once_and_resets_each_second() {
        let mut budget = IntentBudget::default();
        let flooder = ClientId::Netcode(1);
        let calm = ClientId::Netcode(2);
        let burst = vec![flooder; (RELIABLE_BUDGET_PER_SEC + 10) as usize];
        let flagged = budget.charge(0.1, burst.iter().copied().chain([calm]), []);
        assert_eq!(flagged, vec![flooder]);
        assert!(budget.charge(0.2, [flooder], []).is_empty(), "uma vez só");
        assert!(budget.charge(1.5, [flooder], []).is_empty(), "janela nova");
    }

    #[test]
    fn redact_url_hides_password() {
        assert_eq!(
            redact_url("postgres://marvyr:s3nh4@db:5432/marvyr"),
            "postgres://marvyr:***@db:5432/marvyr"
        );
        assert_eq!(redact_url("sem-esquema"), "***");
        assert_eq!(
            redact_url("postgres://db/marvyr?password=x&sslmode=require"),
            "postgres://db/marvyr"
        );
    }
}
