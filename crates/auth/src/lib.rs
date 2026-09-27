//! `marvyr-auth`: serviço HTTP de contas (registro/login) que emite o token
//! de sessão validado pelo game server (`marvyr-auth-token`).

pub mod validate;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;

/// Mesmas migrations do game server — um schema, uma fonte.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

const BODY_LIMIT_BYTES: usize = 4 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RATE_LIMIT_MAX: u32 = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
/// `/v1/web-cert` é público e sem limite por IP: a resposta fica em memória
/// por este tempo, então uma enxurrada de GET não ocupa o pool do login.
const WEB_CERT_CACHE: Duration = Duration::from_secs(10);
/// (quando foi lido, hash do banco — `None` = servidor sem web).
type CachedCert = (Instant, Option<String>);

/// Conecta e aplica as migrations. Qualquer falha derruba a partida (fail closed).
pub async fn connect(database_url: &str) -> Result<PgPool, Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    secret: Arc<[u8]>,
    ttl_secs: u64,
    trust_proxy: bool,
    limiter: Arc<RateLimiter>,
    web_cert: Arc<Mutex<Option<CachedCert>>>,
}

impl AppState {
    /// Recusa segredo fraco aqui para o serviço nunca subir sem conseguir emitir token.
    pub fn new(
        pool: PgPool,
        secret: Vec<u8>,
        ttl_secs: u64,
        trust_proxy: bool,
    ) -> Result<Self, marvyr_auth_token::TokenError> {
        if secret.len() < marvyr_auth_token::MIN_SECRET_LEN {
            return Err(marvyr_auth_token::TokenError::WeakSecret);
        }
        Ok(Self {
            pool,
            secret: secret.into(),
            ttl_secs,
            trust_proxy,
            limiter: Arc::new(RateLimiter::new(RATE_LIMIT_MAX, RATE_LIMIT_WINDOW)),
            web_cert: Arc::default(),
        })
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/register", post(register))
        .route("/v1/login", post(login))
        .route("/v1/web-cert", get(web_cert))
        .route("/healthz", get(healthz))
        .layer(DefaultBodyLimit::max(BODY_LIMIT_BYTES))
        // Build web: a página (itch.io etc.) é outra origem. Qualquer origem
        // é seguro aqui — a API não usa cookie nem credencial implícita; a
        // senha e o token viajam no corpo.
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
                .allow_headers([axum::http::header::CONTENT_TYPE]),
        )
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .with_state(state)
}

// Sem `Debug`: a senha nunca deve parar num log por acidente.
#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct Session {
    token: String,
    username: String,
    account_id: Uuid,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }

    /// Detalhe vai para o log; o cliente recebe só a mensagem genérica.
    fn internal(context: &str, error: impl std::fmt::Display) -> Self {
        tracing::error!("{context}: {error}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "erro interno")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            error: &'static str,
        }
        (
            self.status,
            Json(Body {
                error: self.message,
            }),
        )
            .into_response()
    }
}

const INVALID_LOGIN: &str = "usuário ou senha inválidos";

async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<Credentials>, JsonRejection>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    rate_limit(&state, &headers, peer)?;
    let Json(creds) = body.map_err(json_rejection)?;
    validate::username(&creds.username).map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, m))?;
    validate::password(&creds.password).map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, m))?;

    let password = creds.password;
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| ApiError::internal("tarefa de hash", e))?
        .map_err(|e| ApiError::internal("hash argon2", e))?;

    let account_id = Uuid::new_v4();
    // `email` é NOT NULL UNIQUE no schema: sintetizado do username normalizado.
    let email = format!("{}@players.marvyr", creds.username.to_lowercase());
    let inserted = sqlx::query(
        "INSERT INTO accounts (id, email, password_hash, username) VALUES ($1, $2, $3, $4)",
    )
    .bind(account_id)
    .bind(&email)
    .bind(&hash)
    .bind(&creds.username)
    .execute(&state.pool)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "nome de usuário já está em uso",
            ));
        }
        Err(e) => return Err(ApiError::internal("insert em accounts", e)),
    }

    tracing::info!(%account_id, username = %creds.username, "conta registrada");
    let session = session(&state, account_id, creds.username)?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<Credentials>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    rate_limit(&state, &headers, peer)?;
    let Json(creds) = body.map_err(json_rejection)?;
    // Formato é público: rejeitar cedo não vaza nada e poupa argon2 com lixo.
    if validate::username(&creds.username).is_err() || validate::password(&creds.password).is_err()
    {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, INVALID_LOGIN));
    }

    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, username, password_hash FROM accounts WHERE lower(username) = lower($1)",
    )
    .bind(&creds.username)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::internal("select em accounts", e))?;

    // Usuário inexistente ainda paga um argon2 (hash fictício): o tempo de
    // resposta não diz se o nome existe.
    let (account_id, username, phc) = match row {
        Some((id, name, phc)) => (Some(id), name, Some(phc)),
        None => (None, String::new(), None),
    };
    let password = creds.password;
    let ok = tokio::task::spawn_blocking(move || match &phc {
        Some(phc) => verify_password(&password, phc),
        None => verify_password(&password, dummy_hash()),
    })
    .await
    .map_err(|e| ApiError::internal("tarefa de verificação", e))?;

    match account_id {
        Some(account_id) if ok => Ok(Json(session(&state, account_id, username)?)),
        _ => Err(ApiError::new(StatusCode::UNAUTHORIZED, INVALID_LOGIN)),
    }
}

#[derive(Serialize)]
struct WebCert {
    digest: String,
}

/// Hash do certificado WebTransport que o game server publicou no boot. Só
/// vale enquanto o certificado vale (13 dias); depois, 404 até reiniciar.
async fn web_cert(State(state): State<AppState>) -> Result<Json<WebCert>, ApiError> {
    let cached = state
        .web_cert
        .lock()
        .ok()
        .and_then(|cache| cache.clone())
        .filter(|(at, _)| at.elapsed() < WEB_CERT_CACHE);
    let digest = match cached {
        Some((_, digest)) => digest,
        None => {
            let digest: Option<String> = sqlx::query_scalar(
                "SELECT digest FROM web_cert WHERE updated_at > now() - interval '13 days'",
            )
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| ApiError::internal("leitura do certificado web", e))?;
            if let Ok(mut cache) = state.web_cert.lock() {
                *cache = Some((Instant::now(), digest.clone()));
            }
            digest
        }
    };
    digest
        .map(|digest| Json(WebCert { digest }))
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "servidor sem acesso pelo navegador"))
}

async fn healthz(State(state): State<AppState>) -> Result<&'static str, ApiError> {
    sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::warn!("healthz sem banco: {e}");
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "banco indisponível")
        })?;
    Ok("ok")
}

fn session(state: &AppState, account_id: Uuid, username: String) -> Result<Session, ApiError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ApiError::internal("relógio", e))?
        .as_secs();
    let token = marvyr_auth_token::issue(account_id, &username, &state.secret, now, state.ttl_secs)
        .map_err(|e| ApiError::internal("emissão de token", e))?;
    Ok(Session {
        token,
        username,
        account_id,
    })
}

fn json_rejection(rejection: JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "requisição grande demais");
    }
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "corpo inválido: esperado JSON {username, password}",
    )
}

fn rate_limit(state: &AppState, headers: &HeaderMap, peer: SocketAddr) -> Result<(), ApiError> {
    let ip = client_ip(headers, peer, state.trust_proxy);
    if state.limiter.check(ip, Instant::now()) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "muitas tentativas; aguarde um minuto",
        ))
    }
}

/// `X-Forwarded-For` só vale atrás do proxy (Traefik); direto na internet o
/// cliente forjaria o header para escapar do rate limit. Usa a entrada mais
/// à direita — a que o nosso proxy (um salto) anexou; as da esquerda vêm do
/// cliente.
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr, trust_proxy: bool) -> IpAddr {
    if trust_proxy {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    peer.ip()
}

fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
        .unwrap_or(false)
}

fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password("hash-ficticio-para-tempo-constante").expect("argon2"))
}

// ponytail: rate limit em memória, por réplica — com N réplicas o teto vira
// N×10/min por IP e reinicia no deploy. Mover para Redis/Traefik middleware
// se escalar horizontalmente.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    hits: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl RateLimiter {
    /// Acima disso, entradas vencidas são podadas (varredura O(n) por request
    /// enquanto houver muitos IPs ativos na mesma janela).
    const PRUNE_AT: usize = 4096;

    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Janela fixa: `true` se a tentativa cabe no limite (e já a contabiliza).
    pub fn check(&self, ip: IpAddr, now: Instant) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|p| p.into_inner());
        if hits.len() >= Self::PRUNE_AT {
            let window = self.window;
            hits.retain(|_, (start, _)| now.duration_since(*start) < window);
        }
        let entry = hits.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        if entry.1 >= self.max {
            return false;
        }
        entry.1 += 1;
        true
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

/// Forma segura de logar a URL do banco: sem senha e sem query string.
pub fn redact_database_url(url: &str) -> String {
    let url = url.split('?').next().unwrap_or_default();
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<url inválida>".to_owned();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    match authority.rsplit_once('@') {
        Some((userinfo, host)) => {
            let user = userinfo.split(':').next().unwrap_or_default();
            format!("{scheme}://{user}:***@{host}{path}")
        }
        None => format!("{scheme}://{authority}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_blocks_after_max_and_resets_after_window() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let other: IpAddr = "10.0.0.2".parse().unwrap();
        let t0 = Instant::now();
        assert!(limiter.check(ip, t0));
        assert!(limiter.check(ip, t0));
        assert!(limiter.check(ip, t0));
        assert!(!limiter.check(ip, t0 + Duration::from_secs(59)));
        assert!(limiter.check(other, t0), "limite é por IP");
        assert!(limiter.check(ip, t0 + Duration::from_secs(60)));
    }

    #[test]
    fn limiter_prunes_expired_entries() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let t0 = Instant::now();
        for i in 0..RateLimiter::PRUNE_AT as u32 {
            limiter.check(IpAddr::from(i.to_be_bytes()), t0);
        }
        assert_eq!(limiter.len(), RateLimiter::PRUNE_AT);
        limiter.check("1.2.3.4".parse().unwrap(), t0 + Duration::from_secs(61));
        assert_eq!(limiter.len(), 1);
    }

    #[test]
    fn forwarded_for_only_when_trusted() {
        let peer: SocketAddr = "172.18.0.2:5000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.9.9.9, 203.0.113.7".parse().unwrap());
        assert_eq!(client_ip(&headers, peer, false), peer.ip());
        assert_eq!(
            client_ip(&headers, peer, true),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        headers.insert("x-forwarded-for", "lixo".parse().unwrap());
        assert_eq!(client_ip(&headers, peer, true), peer.ip());
    }

    #[test]
    fn password_hash_roundtrip() {
        let phc = hash_password("senha-forte-123").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("senha-forte-123", &phc));
        assert!(!verify_password("senha-errada-123", &phc));
        assert!(
            !verify_password("qualquer", ""),
            "hash vazio (conta legada) nunca loga"
        );
    }

    #[test]
    fn database_url_is_redacted() {
        assert_eq!(
            redact_database_url("postgres://marvyr:s3cr3t@db:5432/marvyr?sslmode=require"),
            "postgres://marvyr:***@db:5432/marvyr"
        );
        assert_eq!(
            redact_database_url("postgres://db/marvyr"),
            "postgres://db/marvyr"
        );
        assert_eq!(redact_database_url("lixo"), "<url inválida>");
    }
}
