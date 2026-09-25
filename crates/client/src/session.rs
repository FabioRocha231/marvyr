//! Sessão do jogador (MV-061): login/registro na conta Marvyr, conexão com
//! o servidor e a tela que explica o que está acontecendo — o jogador nunca
//! fica olhando uma janela que parece travada.
//!
//! Fluxo: tela de login → `marvyr-auth` devolve o JWT → resolve o servidor
//! (DNS) → conecta → `ClientHello` com o JWT → `ServerWelcome`. Sem serviço
//! de contas configurado (dev), entra direto com a identidade anônima.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use lightyear::prelude::client::*;
use marvyr_protocol::{
    ServerWelcome, BUILD_SHA, PROTOCOL_VERSION, REASON_BAD_SESSION, REASON_NO_SESSION,
    VERSION_LABEL,
};
use serde::{Deserialize, Serialize};

use crate::config::{LaunchConfig, ServerTarget, Sources};
use crate::net::ClientIdentity;
use crate::ui;

/// Segundos para o servidor responder antes de "Servidor indisponível".
pub const HANDSHAKE_TIMEOUT_SECS: f32 = 10.0;

/// Testes com transporte injetado: conecta no boot, sem tela.
#[derive(Resource)]
pub struct ConnectImmediately;

/// Onde a sessão está. A UI de conexão desenha este estado.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub enum ConnectionStatus {
    /// Tela de login (com o motivo de ter voltado para ela, se houver).
    #[default]
    Login,
    /// Conta sendo validada no `marvyr-auth`.
    Authenticating,
    Connecting {
        since: f32,
    },
    InGame,
    /// Estava jogando e caiu.
    Lost,
    Unavailable(String),
    Incompatible {
        server_protocol: u16,
        reason: String,
    },
    Rejected(String),
    NotConfigured(String),
}

impl ConnectionStatus {
    pub fn from_rejection(welcome: &ServerWelcome) -> Self {
        if welcome.protocol_version != PROTOCOL_VERSION {
            Self::Incompatible {
                server_protocol: welcome.protocol_version,
                reason: welcome.reason.clone(),
            }
        } else {
            Self::Rejected(welcome.reason.clone())
        }
    }

    /// Queda de conexão: jogando = "perdida"; tentando = "indisponível";
    /// recusas mantêm o motivo que o servidor deu.
    pub fn on_disconnect(&mut self) {
        match self {
            Self::InGame => *self = Self::Lost,
            Self::Connecting { .. } => {
                *self = Self::Unavailable(String::from(
                    "O servidor não respondeu. Ele pode estar offline ou bloqueado pela rede.",
                ));
            }
            _ => {}
        }
    }

    /// Texto da tela de conexão (`None` = em jogo, sem tela).
    pub fn message(&self, target: Option<&ServerTarget>) -> Option<(String, Color)> {
        use crate::i18n::{tr, trf};
        let server = target.map(ToString::to_string).unwrap_or_default();
        let retry = format!("\n\n{}", tr("Enter tenta de novo  ·  Esc sai"));
        let quit = tr("Esc sai");
        Some(match self {
            Self::InGame | Self::Login => return None,
            Self::Authenticating => (tr("Verificando a conta…"), ui::TEXT),
            Self::Connecting { .. } => (trf("Conectando a {0}…", &[&server]), ui::TEXT),
            Self::Lost => (
                format!(
                    "{}{retry}",
                    tr("Conexão perdida.\nSeu navio fica 60 s no mar antes de ancorar no porto.")
                ),
                ui::AMBER,
            ),
            Self::Unavailable(reason) => (
                format!(
                    "{}\n{}{retry}",
                    trf("Servidor indisponível ({0}).", &[&server]),
                    tr(reason)
                ),
                ui::DANGER,
            ),
            Self::Incompatible { server_protocol, .. } => (
                format!(
                    "{}\n\nClient protocol: {PROTOCOL_VERSION}\nServer protocol: {server_protocol}\n\n{quit}",
                    tr("Esta versão do Marvyr está desatualizada.\n\nAtualize o jogo pelo itch.io.")
                ),
                ui::DANGER,
            ),
            Self::Rejected(reason) => (
                format!("{}\n{}{retry}", tr("Conexão recusada."), tr(reason)),
                ui::DANGER,
            ),
            Self::NotConfigured(reason) => (
                format!(
                    "{}\n{}\n\n{}\nserver = \"host:porta\"\n\n{quit}",
                    tr("Servidor do Marvyr não configurado."),
                    tr(reason),
                    tr("Crie marvyr.toml ao lado do executável do jogo com:")
                ),
                ui::DANGER,
            ),
        })
    }
}

/// Configuração resolvida no boot.
#[derive(Resource, Debug, Clone)]
pub struct Launch(pub LaunchConfig);

/// Sessão salva ("lembrar-me"): o JWT volta a ser usado no próximo boot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedSession {
    pub username: String,
    pub token: String,
}

fn session_path() -> std::path::PathBuf {
    crate::config::data_dir().join("session.json")
}

fn load_session() -> Option<SavedSession> {
    serde_json::from_str(&std::fs::read_to_string(session_path()).ok()?).ok()
}

fn save_session(session: &SavedSession) {
    let path = session_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(json) = serde_json::to_string(session) else {
        return;
    };
    // O token é credencial: só o dono lê (0600 em unix; `%APPDATA%` já é
    // do usuário no Windows).
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Ok(mut file) = options.open(path) {
        use std::io::Write;
        let _ = file.write_all(json.as_bytes());
    }
}

fn forget_session() {
    let _ = std::fs::remove_file(session_path());
}

/// Resposta do `marvyr-auth` (`/v1/login` e `/v1/register`).
#[derive(Debug, Deserialize)]
struct AuthOk {
    token: String,
    username: String,
}

#[derive(Debug, Deserialize)]
struct AuthError {
    error: String,
}

/// Login ou registro em curso (thread própria — a janela não congela).
#[derive(Resource, Default)]
struct PendingAuth(Option<Mutex<Receiver<Result<SavedSession, String>>>>);

/// Resolução do servidor em andamento (DNS no nativo, certificado no
/// browser) → config de rede pronta para conectar.
#[derive(Resource, Default)]
struct PendingDns(Option<Mutex<Receiver<DnsResult>>>);

type DnsResult = Result<(ServerTarget, NetConfig), String>;

/// Formulário da tela de login.
#[derive(Resource, Debug, Default, Clone)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
    /// 0 = usuário, 1 = senha.
    pub focus: usize,
    pub message: Option<String>,
    pub remember: bool,
}

#[derive(Component)]
struct ConnectionOverlay;
#[derive(Component)]
struct ConnectionText;
#[derive(Component)]
struct LoginPanel;
#[derive(Component)]
enum LoginField {
    Username,
    Password,
}
#[derive(Component)]
struct LoginMessage;
#[derive(Component, Clone, Copy)]
enum LoginButton {
    Login,
    Register,
    Remember,
    Language,
}
#[derive(Component)]
struct RememberLabel;
#[derive(Component)]
struct LanguageLabel;
#[derive(Component)]
struct RememberBox;

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoginForm>()
            .init_resource::<PendingAuth>()
            .init_resource::<PendingDns>()
            .add_systems(Startup, (boot_session, spawn_screens).chain())
            .add_systems(
                Update,
                (
                    // O Enter que sai da tela de erro não vira submit do login.
                    type_into_form.before(retry_on_enter),
                    click_login_buttons,
                    finish_auth,
                    start_connection,
                    finish_connection,
                    handshake_timeout,
                    retry_on_enter,
                    draw_screens,
                ),
            );
    }
}

/// Resolve a configuração e decide a primeira tela.
fn boot_session(
    mut commands: Commands,
    mut status: ResMut<ConnectionStatus>,
    mut identity: ResMut<ClientIdentity>,
    mut form: ResMut<LoginForm>,
    immediate: Option<Res<ConnectImmediately>>,
) {
    info!(
        version = VERSION_LABEL,
        build = BUILD_SHA,
        protocol = PROTOCOL_VERSION,
        "Marvyr"
    );
    if immediate.is_some() {
        // Testes: `ClientNetPlugin` já conectou com o transporte injetado.
        return;
    }
    let launch = match Sources::from_process().resolve() {
        Ok(launch) => launch,
        Err(error) => {
            *status = ConnectionStatus::NotConfigured(error);
            commands.insert_resource(Launch(LaunchConfig {
                server: None,
                auth_url: None,
            }));
            return;
        }
    };
    info!(
        server = ?launch.server.as_ref().map(ToString::to_string),
        auth = ?launch.auth_url,
        "configuração de lançamento"
    );
    if launch.server.is_none() {
        *status = ConnectionStatus::NotConfigured(String::from(
            "Nenhum servidor foi informado nesta build.",
        ));
    } else if launch.auth_url.is_none() && crate::config::PUBLIC_BUILD {
        *status = ConnectionStatus::NotConfigured(String::from(
            "Nenhum servico de contas (auth_url) foi informado nesta build.",
        ));
    } else if launch.auth_url.is_none() {
        // Dev sem contas: identidade anônima, direto para o mar.
        identity.0 = Some(crate::net::identity_token());
        *status = ConnectionStatus::Authenticating;
    } else if let Some(saved) = load_session() {
        form.username = saved.username.clone();
        form.remember = true;
        identity.0 = Some(saved.token);
        *status = ConnectionStatus::Authenticating;
    } else {
        form.remember = true;
    }
    commands.insert_resource(Launch(launch));
}

/// Com identidade pronta: resolve o servidor (DNS) e conecta.
fn start_connection(
    time: Res<Time>,
    launch: Option<Res<Launch>>,
    identity: Res<ClientIdentity>,
    mut status: ResMut<ConnectionStatus>,
    mut pending: ResMut<PendingDns>,
) {
    if *status != ConnectionStatus::Authenticating || identity.0.is_none() {
        return;
    }
    let Some(launch) = launch else {
        return;
    };
    let Some(target) = launch.0.server.clone() else {
        return;
    };
    // Fora do frame: resolvedor lento não congela a janela. O timeout de
    // 10 s do handshake já conta a partir daqui.
    let (sender, receiver) = channel();
    let done = move |result: DnsResult| {
        let _ = sender.send(result);
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        done(
            target
                .resolve()
                .map(|addr| (target, crate::net::netcode_config(addr))),
        )
    });
    #[cfg(target_arch = "wasm32")]
    web::resolve(target, launch.0.auth_url.clone(), done);
    pending.0 = Some(Mutex::new(receiver));
    *status = ConnectionStatus::Connecting {
        since: time.elapsed_secs(),
    };
}

/// DNS resolvido: abre a conexão (se a tentativa ainda estiver de pé).
fn finish_connection(
    mut commands: Commands,
    mut pending: ResMut<PendingDns>,
    mut status: ResMut<ConnectionStatus>,
    mut config: ResMut<ClientConfig>,
) {
    let Some(receiver) = &pending.0 else {
        return;
    };
    let received = receiver.lock().ok().and_then(|rx| rx.try_recv().ok());
    let Some(result) = received else {
        return;
    };
    pending.0 = None;
    if !matches!(*status, ConnectionStatus::Connecting { .. }) {
        return; // o timeout já desistiu desta tentativa
    }
    match result {
        Ok((target, net)) => {
            info!(server = %target, "conectando ao servidor marvyr");
            config.net = net;
            commands.connect_client();
        }
        Err(error) => {
            warn!(%error, "falha de DNS");
            *status = ConnectionStatus::Unavailable(error);
        }
    }
}

/// Sem `ServerWelcome` em 10 s: desiste e explica.
fn handshake_timeout(
    mut commands: Commands,
    time: Res<Time>,
    mut status: ResMut<ConnectionStatus>,
) {
    if let ConnectionStatus::Connecting { since } = *status {
        if time.elapsed_secs() - since > HANDSHAKE_TIMEOUT_SECS {
            warn!("handshake sem resposta; desistindo");
            commands.disconnect_client();
            *status = ConnectionStatus::Unavailable(String::from(
                "Sem resposta em 10 s. Confira a internet ou tente mais tarde.",
            ));
        }
    }
}

/// Enter numa tela de erro tenta de novo — uma vez por toque, sem laço
/// automático martelando o servidor.
fn retry_on_enter(
    keys: Res<ButtonInput<KeyCode>>,
    mut status: ResMut<ConnectionStatus>,
    mut identity: ResMut<ClientIdentity>,
    mut form: ResMut<LoginForm>,
    launch: Option<Res<Launch>>,
) {
    if !keys.just_pressed(KeyCode::Enter) {
        return;
    }
    match &*status {
        ConnectionStatus::Lost | ConnectionStatus::Unavailable(_) => {
            *status = ConnectionStatus::Authenticating;
        }
        ConnectionStatus::Rejected(reason) => {
            // Sessão ruim volta para o login; o resto tenta de novo.
            let session_problem = (reason == REASON_NO_SESSION || reason == REASON_BAD_SESSION)
                && launch.is_some_and(|launch| launch.0.auth_url.is_some());
            if session_problem {
                forget_session();
                identity.0 = None;
                form.message = Some(reason.clone());
                *status = ConnectionStatus::Login;
            } else {
                *status = ConnectionStatus::Authenticating;
            }
        }
        _ => {}
    }
}

/// Tela de entrada como cartaz de porto: o mar continua vivo atrás, e no
/// centro um cartaz de papel com "MARVYR" em tipo de madeira (a segunda cor
/// fora de registro), o formulário em linhas de preencher e o idioma.
fn spawn_screens(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(ui::INK.with_alpha(0.62)),
            GlobalZIndex(50),
            ConnectionOverlay,
        ))
        .with_children(|root| {
            root.spawn(ui::panel(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                width: Val::Px(480.0),
                max_width: Val::Percent(94.0),
                max_height: Val::Percent(96.0),
                padding: UiRect::axes(Val::Px(34.0), Val::Px(24.0)),
                overflow: Overflow::clip(),
                ..default()
            }))
            .with_children(|poster| {
                ui::masthead(poster, "MARVYR", 76.0);
                ui::double_rule(poster);
                poster.spawn((
                    crate::i18n::label_face(
                        "O transporte arriscado de riqueza fabricada por jogadores",
                        ui::FONT_REGULAR,
                        17.0,
                        ui::INK,
                    ),
                    TextLayout::new_with_justify(JustifyText::Center),
                ));
                poster.spawn((
                    ui::face("", ui::FONT_BOLD, 17.0, ui::INK),
                    TextLayout::new_with_justify(JustifyText::Center),
                    ConnectionText,
                ));
                poster
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            width: Val::Percent(100.0),
                            ..default()
                        },
                        LoginPanel,
                    ))
                    .with_children(|panel| {
                        panel.spawn(crate::i18n::label_face("Capitão", ui::FONT_BOLD, 13.0, ui::INK_SOFT));
                        field(panel, LoginField::Username);
                        panel.spawn(crate::i18n::label_face("Senha", ui::FONT_BOLD, 13.0, ui::INK_SOFT));
                        field(panel, LoginField::Password);
                        panel
                            .spawn(Node {
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                margin: UiRect::top(Val::Px(4.0)),
                                ..default()
                            })
                            .with_children(|row| {
                                row.spawn((
                                    ui::button(
                                        Node {
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        },
                                        ui::PAPER,
                                    ),
                                    LoginButton::Remember,
                                ))
                                .with_children(|button| {
                                    button.spawn((
                                        Node {
                                            width: Val::Px(14.0),
                                            height: Val::Px(14.0),
                                            border: UiRect::all(Val::Px(2.0)),
                                            ..default()
                                        },
                                        BorderColor(ui::INK),
                                        BackgroundColor(Color::NONE),
                                        RememberBox,
                                    ));
                                    button.spawn((
                                        crate::i18n::label("Lembrar de mim", 14.0, ui::INK),
                                        RememberLabel,
                                    ));
                                });
                                row.spawn((
                                    ui::button(Node::default(), ui::PAPER),
                                    LoginButton::Language,
                                ))
                                .with_child((ui::text("", 14.0, ui::INK), LanguageLabel));
                            });
                        panel
                            .spawn(Node {
                                column_gap: Val::Px(12.0),
                                justify_content: JustifyContent::Center,
                                margin: UiRect::top(Val::Px(6.0)),
                                ..default()
                            })
                            .with_children(|row| {
                                row.spawn((
                                    ui::button(
                                        Node {
                                            padding: UiRect::axes(Val::Px(22.0), Val::Px(9.0)),
                                            ..default()
                                        },
                                        ui::BUTTON_SELECTED,
                                    ),
                                    LoginButton::Login,
                                ))
                                .with_child(crate::i18n::label_face(
                                    "Entrar",
                                    ui::FONT_BOLD,
                                    18.0,
                                    ui::INK,
                                ));
                                row.spawn((
                                    ui::button(
                                        Node {
                                            padding: UiRect::axes(Val::Px(18.0), Val::Px(9.0)),
                                            ..default()
                                        },
                                        ui::PAPER_SHADE,
                                    ),
                                    LoginButton::Register,
                                ))
                                .with_child(crate::i18n::label_face(
                                    "Criar conta",
                                    ui::FONT_BOLD,
                                    18.0,
                                    ui::INK,
                                ));
                            });
                        panel.spawn((
                            ui::face("", ui::FONT_BOLD, 14.0, ui::VERMILION_INK),
                            TextLayout::new_with_justify(JustifyText::Center),
                            LoginMessage,
                        ));
                        panel.spawn((
                            crate::i18n::label_face(
                                "Tab troca de campo · nome: 3 a 20 letras, números ou _ · senha: 8+",
                                ui::FONT_REGULAR,
                                13.0,
                                ui::INK_SOFT,
                            ),
                            TextLayout::new_with_justify(JustifyText::Center),
                        ));
                    });
                ui::rule(poster);
                poster.spawn(ui::text(
                    format!(
                        "Marvyr {VERSION_LABEL} · build {BUILD_SHA} · protocolo {PROTOCOL_VERSION}"
                    ),
                    12.0,
                    ui::INK_SOFT,
                ));
            });
        });
}

/// Campo de preencher: linha de tinta embaixo, como num formulário impresso.
fn field(panel: &mut ChildBuilder, kind: LoginField) {
    panel.spawn((
        Node {
            border: UiRect::bottom(Val::Px(2.0)),
            padding: UiRect::axes(Val::Px(6.0), Val::Px(5.0)),
            min_height: Val::Px(34.0),
            ..default()
        },
        BackgroundColor(ui::PAPER_SHADE.with_alpha(0.6)),
        BorderColor(ui::INK),
        Interaction::None,
        ui::face("", ui::FONT_BOLD, 19.0, ui::INK),
        kind,
    ));
}

/// Digitação no formulário (só na tela de login).
fn type_into_form(
    mut keys: EventReader<KeyboardInput>,
    mut form: ResMut<LoginForm>,
    mut status: ResMut<ConnectionStatus>,
    mut pending: ResMut<PendingAuth>,
    launch: Option<Res<Launch>>,
) {
    if *status != ConnectionStatus::Login {
        keys.clear();
        return;
    }
    for event in keys.read() {
        if !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Tab => form.focus = 1 - form.focus.min(1),
            Key::Backspace => {
                let field = if form.focus == 0 {
                    &mut form.username
                } else {
                    &mut form.password
                };
                field.pop();
            }
            Key::Enter => {
                submit(
                    &mut form,
                    &mut status,
                    &mut pending,
                    launch.as_deref(),
                    false,
                );
            }
            Key::Character(chars) => {
                let field = if form.focus == 0 {
                    &mut form.username
                } else {
                    &mut form.password
                };
                if field.chars().count() < 128 {
                    field.extend(chars.chars().filter(|c| !c.is_control()));
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn click_login_buttons(
    buttons: Query<(&Interaction, &LoginButton), Changed<Interaction>>,
    fields: Query<(&Interaction, &LoginField), Changed<Interaction>>,
    mut form: ResMut<LoginForm>,
    mut status: ResMut<ConnectionStatus>,
    mut pending: ResMut<PendingAuth>,
    launch: Option<Res<Launch>>,
    lang: Res<crate::i18n::Lang>,
    mut change_lang: EventWriter<crate::i18n::ChangeLang>,
) {
    if *status != ConnectionStatus::Login {
        return;
    }
    for (interaction, field) in &fields {
        if *interaction == Interaction::Pressed {
            form.focus = match field {
                LoginField::Username => 0,
                LoginField::Password => 1,
            };
        }
    }
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            LoginButton::Login => submit(
                &mut form,
                &mut status,
                &mut pending,
                launch.as_deref(),
                false,
            ),
            LoginButton::Register => submit(
                &mut form,
                &mut status,
                &mut pending,
                launch.as_deref(),
                true,
            ),
            LoginButton::Remember => form.remember = !form.remember,
            LoginButton::Language => {
                change_lang.send(crate::i18n::ChangeLang(lang.toggled()));
            }
        }
    }
}

/// Validação local igual à do servidor de contas (feedback imediato; a
/// palavra final é do `marvyr-auth`).
pub fn validate_form(username: &str, password: &str) -> Result<(), &'static str> {
    let name_ok = (3..=20).contains(&username.chars().count())
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !name_ok {
        return Err("Nome de capitão: 3 a 20 letras, números ou _.");
    }
    if !(8..=128).contains(&password.chars().count()) {
        return Err("A senha precisa ter de 8 a 128 caracteres.");
    }
    Ok(())
}

fn submit(
    form: &mut LoginForm,
    status: &mut ConnectionStatus,
    pending: &mut PendingAuth,
    launch: Option<&Launch>,
    register: bool,
) {
    let Some(auth_url) = launch.and_then(|launch| launch.0.auth_url.clone()) else {
        return;
    };
    if let Err(reason) = validate_form(form.username.trim(), &form.password) {
        form.message = Some(reason.to_owned());
        return;
    }
    let username = form.username.trim().to_owned();
    let password = std::mem::take(&mut form.password);
    let (tx, rx) = channel();
    let done = move |result: Result<SavedSession, String>| {
        let _ = tx.send(result);
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || done(call_auth(&auth_url, register, &username, &password)));
    #[cfg(target_arch = "wasm32")]
    web::call_auth(&auth_url, register, &username, &password, done);
    pending.0 = Some(Mutex::new(rx));
    form.message = None;
    *status = ConnectionStatus::Authenticating;
}

/// Chamada HTTP ao `marvyr-auth` (bloqueante, fora da thread do jogo).
#[cfg(not(target_arch = "wasm32"))]
fn call_auth(
    auth_url: &str,
    register: bool,
    username: &str,
    password: &str,
) -> Result<SavedSession, String> {
    let route = if register { "register" } else { "login" };
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let response = agent
        .post(&format!("{auth_url}/v1/{route}"))
        .send_json(serde_json::json!({ "username": username, "password": password }));
    match response {
        Ok(response) => {
            let ok: AuthOk = response
                .into_json()
                .map_err(|_| String::from("Resposta inválida do servidor de contas."))?;
            Ok(SavedSession {
                username: ok.username,
                token: ok.token,
            })
        }
        Err(ureq::Error::Status(_, response)) => Err(response
            .into_json::<AuthError>()
            .map(|body| body.error)
            .unwrap_or_else(|_| String::from("O servidor de contas recusou o pedido."))),
        Err(error) => Err(format!("Servidor de contas indisponível: {error}")),
    }
}

fn finish_auth(
    mut pending: ResMut<PendingAuth>,
    mut form: ResMut<LoginForm>,
    mut status: ResMut<ConnectionStatus>,
    mut identity: ResMut<ClientIdentity>,
) {
    let result = {
        let Some(receiver) = &pending.0 else {
            return;
        };
        let Ok(receiver) = receiver.lock() else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(String::from(
                "Falha inesperada ao falar com o servidor de contas.",
            )),
        }
    };
    pending.0 = None;
    match result {
        Ok(session) => {
            info!(captain = %session.username, "conta autenticada");
            if form.remember {
                save_session(&session);
            } else {
                forget_session();
            }
            form.username = session.username.clone();
            identity.0 = Some(session.token);
            // `start_connection` segue no próximo frame (Authenticating).
        }
        Err(reason) => {
            form.message = Some(reason);
            *status = ConnectionStatus::Login;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw_screens(
    status: Res<ConnectionStatus>,
    form: Res<LoginForm>,
    launch: Option<Res<Launch>>,
    mut overlay: Query<&mut Visibility, (With<ConnectionOverlay>, Without<LoginPanel>)>,
    mut panel: Query<&mut Visibility, (With<LoginPanel>, Without<ConnectionOverlay>)>,
    mut texts: Query<
        (&mut Text, &mut TextColor),
        (
            With<ConnectionText>,
            Without<LoginField>,
            Without<LoginMessage>,
            Without<RememberLabel>,
        ),
    >,
    mut fields: Query<(&LoginField, &mut Text, &mut BorderColor), Without<ConnectionText>>,
    mut messages: Query<
        &mut Text,
        (
            With<LoginMessage>,
            Without<ConnectionText>,
            Without<LoginField>,
            Without<RememberLabel>,
        ),
    >,
    mut remember: Query<&mut BackgroundColor, With<RememberBox>>,
    mut language: Query<
        &mut Text,
        (
            With<LanguageLabel>,
            Without<ConnectionText>,
            Without<LoginField>,
            Without<LoginMessage>,
            Without<RememberLabel>,
        ),
    >,
    lang: Res<crate::i18n::Lang>,
) {
    if !status.is_changed() && !form.is_changed() && !lang.is_changed() {
        return;
    }
    let in_game = *status == ConnectionStatus::InGame;
    let login = *status == ConnectionStatus::Login;
    for mut visibility in &mut overlay {
        *visibility = if in_game {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    for mut visibility in &mut panel {
        *visibility = if login {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    let target = launch.as_ref().and_then(|launch| launch.0.server.as_ref());
    for (mut text, mut color) in &mut texts {
        let (message, tint) = status.message(target).unwrap_or_default();
        text.0 = crate::i18n::tr(&message);
        color.0 = tint;
    }
    for (kind, mut text, mut border) in &mut fields {
        let (value, focused) = match kind {
            LoginField::Username => (form.username.clone(), form.focus == 0),
            LoginField::Password => ("*".repeat(form.password.chars().count()), form.focus == 1),
        };
        text.0 = if focused { format!("{value}|") } else { value };
        border.0 = if focused { ui::VERMILION } else { ui::INK };
    }
    for mut text in &mut messages {
        text.0 = crate::i18n::tr(&form.message.clone().unwrap_or_default());
    }
    for mut check in &mut remember {
        check.0 = if form.remember { ui::INK } else { Color::NONE };
    }
    for mut text in &mut language {
        // O botão oferece o OUTRO idioma, escrito nele mesmo.
        text.0 = String::from(match *lang {
            crate::i18n::Lang::Pt => "Play in English",
            crate::i18n::Lang::En => "Jogar em português",
        });
    }
}

/// Browser: `fetch` no lugar da thread + ureq/DNS do nativo. O callback do
/// `ehttp` roda fora do frame e devolve pelo mesmo canal do nativo.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::{AuthError, AuthOk, DnsResult, SavedSession, ServerTarget};

    /// Resposta de `GET /v1/web-cert` do `marvyr-auth`.
    #[derive(serde::Deserialize)]
    struct WebCert {
        digest: String,
    }

    pub fn call_auth(
        auth_url: &str,
        register: bool,
        username: &str,
        password: &str,
        done: impl FnOnce(Result<SavedSession, String>) + Send + 'static,
    ) {
        let route = if register { "register" } else { "login" };
        let body = serde_json::json!({ "username": username, "password": password });
        let Ok(request) = ehttp::Request::json(format!("{auth_url}/v1/{route}"), &body) else {
            return done(Err(String::from("Falha ao montar o pedido de login.")));
        };
        ehttp::fetch(request, move |response| {
            done(match response {
                Ok(response) if response.ok => response
                    .json::<AuthOk>()
                    .map(|ok| SavedSession {
                        username: ok.username,
                        token: ok.token,
                    })
                    .map_err(|_| String::from("Resposta inválida do servidor de contas.")),
                Ok(response) => Err(response
                    .json::<AuthError>()
                    .map(|body| body.error)
                    .unwrap_or_else(|_| String::from("O servidor de contas recusou o pedido."))),
                Err(error) => Err(format!("Servidor de contas indisponível: {error}")),
            })
        });
    }

    /// Busca o hash do certificado WebTransport do servidor e monta a
    /// conexão. O endereço tem de ser IPv4 literal: o browser não resolve
    /// DNS para o wasm.
    pub fn resolve(
        target: ServerTarget,
        auth_url: Option<String>,
        done: impl FnOnce(DnsResult) + Send + 'static,
    ) {
        let finish = move |digest: Result<String, String>| {
            done(digest.and_then(|digest| {
                let addr = target.resolve()?;
                Ok((target, crate::net::web_netcode_config(addr, digest)))
            }))
        };
        // Dev sem `marvyr-auth`: `?cert=<hex>` do log do servidor. Nunca na
        // build pública — um link malicioso fixaria o certificado de outro.
        if !crate::config::PUBLIC_BUILD {
            if let Some(digest) = query_param("cert") {
                return finish(Ok(digest));
            }
        }
        let Some(auth_url) = auth_url else {
            return finish(Err(String::from(
                "Sem serviço de contas para buscar o certificado do servidor.",
            )));
        };
        let request = ehttp::Request::get(format!("{auth_url}/v1/web-cert"));
        ehttp::fetch(request, move |response| {
            finish(match response {
                Ok(response) if response.ok => response
                    .json::<WebCert>()
                    .map(|cert| cert.digest)
                    .map_err(|_| String::from("Resposta inválida do servidor de contas.")),
                Ok(response) => Err(format!(
                    "Certificado do servidor indisponível (HTTP {}).",
                    response.status
                )),
                Err(error) => Err(format!("Servidor de contas indisponível: {error}")),
            })
        });
    }

    fn query_param(name: &str) -> Option<String> {
        let search = web_sys::window()?.location().search().ok()?;
        search.trim_start_matches('?').split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name && !value.is_empty()).then(|| value.to_owned())
        })
    }
}

#[cfg(test)]
pub(crate) fn init_systems_for_tests(world: &mut World) {
    fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
        let mut system = IntoSystem::into_system(system);
        system.initialize(world);
    }
    init(world, draw_screens);
    init(world, click_login_buttons);
    init(world, type_into_form);
    init(world, start_connection);
    init(world, finish_connection);
    init(world, handshake_timeout);
    init(world, retry_on_enter);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_validation_matches_account_rules() {
        assert!(validate_form("barba_ruiva", "segredo123").is_ok());
        assert!(validate_form("ab", "segredo123").is_err());
        assert!(validate_form("nome com espaço", "segredo123").is_err());
        assert!(validate_form("capitao", "curta").is_err());
    }

    #[test]
    fn protocol_mismatch_is_not_a_generic_rejection() {
        let incompatible = ConnectionStatus::from_rejection(&ServerWelcome {
            protocol_version: PROTOCOL_VERSION + 1,
            accepted: false,
            reason: String::from("desatualizada"),
        });
        let (text, _) = incompatible.message(None).unwrap();
        assert!(text.contains("desatualizada"));
        assert!(text.contains(&format!("Client protocol: {PROTOCOL_VERSION}")));
        assert!(text.contains(&format!("Server protocol: {}", PROTOCOL_VERSION + 1)));
        let full = ConnectionStatus::from_rejection(&ServerWelcome::rejected("Servidor cheio."));
        assert_eq!(
            full,
            ConnectionStatus::Rejected(String::from("Servidor cheio."))
        );
    }

    #[test]
    fn disconnect_while_connecting_means_server_unavailable() {
        let mut status = ConnectionStatus::Connecting { since: 0.0 };
        status.on_disconnect();
        assert!(matches!(status, ConnectionStatus::Unavailable(_)));
        let mut status = ConnectionStatus::InGame;
        status.on_disconnect();
        assert_eq!(status, ConnectionStatus::Lost);
        let mut status = ConnectionStatus::Rejected(String::from("x"));
        status.on_disconnect();
        assert_eq!(status, ConnectionStatus::Rejected(String::from("x")));
    }

    #[test]
    fn in_game_has_no_overlay() {
        assert!(ConnectionStatus::InGame.message(None).is_none());
        assert!(ConnectionStatus::NotConfigured(String::new())
            .message(None)
            .unwrap()
            .0
            .contains("não configurado"));
    }
}
