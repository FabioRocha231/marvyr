//! Rotas do `marvyr-auth` via `oneshot` (sem socket).
//!
//! Validação/rate limit rodam sem banco (pool preguiçoso que nunca conecta).
//! O fluxo completo roda contra PostgreSQL real:
//!
//! ```text
//! docker run --rm -d --name marvyr-pg -p 54329:5432 \
//!   -e POSTGRES_PASSWORD=marvyr postgres:16-alpine
//! MARVYR_TEST_DATABASE_URL=postgres://postgres:marvyr@localhost:54329/postgres \
//!   cargo test -p marvyr-auth --test http
//! ```
//!
//! Sem a variável, o teste de banco pula com aviso (reporte honesto > falso verde).

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use marvyr_auth::{app, connect, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &[u8] = b"segredo-de-teste-com-pelo-menos-32-bytes";

fn router(pool: sqlx::PgPool, peer: &str) -> Router {
    let state = AppState::new(pool, SECRET.to_vec(), 3600, false).expect("estado");
    let peer: SocketAddr = peer.parse().unwrap();
    app(state).layer(MockConnectInfo(peer))
}

fn lazy_router() -> Router {
    // Nunca conecta: só serve para rotas que falham antes do banco.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://ninguem@127.0.0.1:1/nada")
        .unwrap();
    router(pool, "10.0.0.1:4000")
}

async fn call(router: &Router, path: &str, body: &str) -> (StatusCode, Value) {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn weak_secret_is_refused() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://ninguem@127.0.0.1:1/nada")
        .unwrap();
    assert!(AppState::new(pool, b"curto".to_vec(), 3600, false).is_err());
}

#[tokio::test]
async fn register_validation_returns_400_json() {
    let router = lazy_router();
    for body in [
        json!({"username": "ab", "password": "senha-boa-123"}),
        json!({"username": "nome com espaco", "password": "senha-boa-123"}),
        json!({"username": "capitao", "password": "curta"}),
    ] {
        let (status, value) = call(&router, "/v1/register", &body.to_string()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(value["error"].is_string(), "erro em JSON: {value}");
    }
    let (status, value) = call(&router, "/v1/register", "{nao-e-json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(value["error"].is_string());
}

#[tokio::test]
async fn oversized_body_returns_413() {
    let router = lazy_router();
    let body = json!({"username": "capitao", "password": "x".repeat(8 * 1024)}).to_string();
    let (status, value) = call(&router, "/v1/login", &body).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(value["error"].is_string());
}

#[tokio::test]
async fn rate_limit_returns_429_after_ten_attempts() {
    let router = lazy_router();
    let body = json!({"username": "ab", "password": "x"}).to_string();
    for _ in 0..10 {
        let (status, _) = call(&router, "/v1/register", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, value) = call(&router, "/v1/login", &body).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "login e registro somam"
    );
    assert!(value["error"].is_string());
}

#[tokio::test]
async fn browser_preflight_is_allowed() {
    let request = Request::options("/v1/login")
        .header("origin", "https://marvyr.itch.io")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .body(Body::empty())
        .unwrap();
    let response = lazy_router().oneshot(request).await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "*",
        "página web (outra origem) precisa poder chamar o login"
    );
}

#[tokio::test]
async fn full_flow_against_postgres() {
    let Ok(url) = std::env::var("MARVYR_TEST_DATABASE_URL") else {
        eprintln!("PULANDO: defina MARVYR_TEST_DATABASE_URL para testar o fluxo do marvyr-auth");
        return;
    };
    let pool = connect(&url)
        .await
        .unwrap_or_else(|e| panic!("banco de teste configurado mas não abriu: {e}"));
    let router = router(pool.clone(), "10.9.9.9:4000");
    // Nome único por execução: não depende de banco limpo.
    let username = format!("Cap_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let creds = json!({"username": username, "password": "senha-forte-123"}).to_string();

    let (status, registered) = call(&router, "/v1/register", &creds).await;
    assert_eq!(status, StatusCode::CREATED, "{registered}");
    assert_eq!(registered["username"], username);
    let account_id: uuid::Uuid = registered["account_id"].as_str().unwrap().parse().unwrap();

    // Login é case-insensitive no nome e devolve o nome como registrado.
    let login = json!({"username": username.to_lowercase(), "password": "senha-forte-123"});
    let (status, session) = call(&router, "/v1/login", &login.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["username"], username);
    let claims =
        marvyr_auth_token::verify(session["token"].as_str().unwrap(), SECRET).expect("token");
    assert_eq!(claims.sub, account_id);
    assert_eq!(claims.username, username);

    let wrong = json!({"username": username, "password": "senha-errada-123"}).to_string();
    let (status, wrong_pw) = call(&router, "/v1/login", &wrong).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let ghost = json!({"username": "ninguem_aqui_x", "password": "senha-forte-123"}).to_string();
    let (status, no_user) = call(&router, "/v1/login", &ghost).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong_pw, no_user,
        "mesma resposta: não vaza se o nome existe"
    );

    let upper = json!({"username": username.to_uppercase(), "password": "outra-senha-123"});
    let (status, conflict) = call(&router, "/v1/register", &upper.to_string()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    // Certificado web publicado pelo game server chega ao browser.
    let digest = "ab".repeat(32);
    sqlx::query(
        "INSERT INTO web_cert (id, digest, updated_at) VALUES (TRUE, $1, now()) \
         ON CONFLICT (id) DO UPDATE SET digest = EXCLUDED.digest, updated_at = EXCLUDED.updated_at",
    )
    .bind(&digest)
    .execute(&pool)
    .await
    .unwrap();
    let request = Request::get("/v1/web-cert").body(Body::empty()).unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let cert: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cert["digest"], digest.as_str());

    let health = Request::get("/healthz").body(Body::empty()).unwrap();
    assert_eq!(
        router.clone().oneshot(health).await.unwrap().status(),
        StatusCode::OK
    );
    pool.close().await;
}
