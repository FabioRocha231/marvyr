//! Configuração de lançamento do client (MV-061): onde fica o servidor, o
//! serviço de contas, os assets e os dados do jogador.
//!
//! Prioridade do servidor: `--server host:porta` → `MARVYR_SERVER_HOST` +
//! `MARVYR_PORT` → `marvyr.toml` ao lado do executável → padrão embutido no
//! build (`MARVYR_DEFAULT_SERVER`) → `127.0.0.1:5000` só em build de dev.
//! Build pública (`MARVYR_PUBLIC_BUILD=1`) nunca cai silenciosamente em
//! localhost.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};

/// UDP no nativo; no browser, a do WebTransport (`MARVYR_WEB_PORT` do
/// servidor) — QUIC também é UDP e não divide a porta com o netcode.
pub const DEFAULT_PORT: u16 = if cfg!(target_arch = "wasm32") {
    5001
} else {
    5000
};

/// Build distribuída para jogadores (sem fallback de desenvolvimento).
pub const PUBLIC_BUILD: bool = option_env!("MARVYR_PUBLIC_BUILD").is_some();

/// Servidor e contas embutidos pelo pipeline de release.
const BAKED_SERVER: Option<&str> = option_env!("MARVYR_DEFAULT_SERVER");
const BAKED_AUTH_URL: Option<&str> = option_env!("MARVYR_DEFAULT_AUTH_URL");

/// Destino do jogo como o jogador/operador escreveu (`host:porta`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerTarget {
    pub host: String,
    pub port: u16,
}

impl std::fmt::Display for ServerTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl ServerTarget {
    /// Resolve hostname/IP (DNS incluso) para o primeiro endereço IPv4 —
    /// o socket local é `0.0.0.0:0`.
    pub fn resolve(&self) -> Result<SocketAddr, String> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|error| format!("não foi possível resolver {self}: {error}"))?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| format!("{self} não tem endereço IPv4"))
    }
}

/// Configuração resolvida no boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `None` = nenhum servidor configurado (erro claro na build pública).
    pub server: Option<ServerTarget>,
    /// `None` = sem serviço de contas: dev entra com identidade anônima.
    pub auth_url: Option<String>,
}

/// Entradas cruas de cada fonte (testável sem mexer no processo).
#[derive(Debug, Default)]
pub struct Sources {
    pub cli_server: Option<String>,
    pub cli_auth: Option<String>,
    pub env_host: Option<String>,
    pub env_port: Option<String>,
    pub env_auth: Option<String>,
    pub file: Option<String>,
    pub baked_server: Option<String>,
    pub baked_auth: Option<String>,
    pub public_build: bool,
}

impl Sources {
    pub fn from_process() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let flag = |name: &str| {
            args.iter()
                .position(|arg| arg == name)
                .and_then(|index| args.get(index + 1).cloned())
                .or_else(|| {
                    let prefix = format!("{name}=");
                    args.iter()
                        .find_map(|arg| arg.strip_prefix(&prefix).map(str::to_owned))
                })
        };
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            cli_server: flag("--server"),
            cli_auth: flag("--auth-url"),
            env_host: env("MARVYR_SERVER_HOST"),
            env_port: env("MARVYR_PORT"),
            env_auth: env("MARVYR_AUTH_URL"),
            file: exe_dir().and_then(|dir| std::fs::read_to_string(dir.join("marvyr.toml")).ok()),
            // Variável do CI vazia chega como `Some("")`: vale como ausente.
            baked_server: BAKED_SERVER
                .filter(|v| !v.trim().is_empty())
                .map(str::to_owned),
            baked_auth: BAKED_AUTH_URL
                .filter(|v| !v.trim().is_empty())
                .map(str::to_owned),
            public_build: PUBLIC_BUILD,
        }
    }

    pub fn resolve(&self) -> Result<LaunchConfig, String> {
        let file_server = self
            .file
            .as_deref()
            .and_then(|file| toml_value(file, "server"));
        let file_auth = self
            .file
            .as_deref()
            .and_then(|file| toml_value(file, "auth_url"));
        // Só lida quando é ela que decide a porta (env ou dev).
        let env_port = || -> Result<u16, String> {
            self.env_port
                .as_deref()
                .map(parse_port)
                .transpose()
                .map(|port| port.unwrap_or(DEFAULT_PORT))
        };
        let server = if let Some(raw) = &self.cli_server {
            Some(parse_target(raw)?)
        } else if let Some(host) = &self.env_host {
            Some(ServerTarget {
                host: host.trim().to_owned(),
                port: env_port()?,
            })
        } else if let Some(raw) = file_server.or(self.baked_server.clone()) {
            Some(parse_target(&raw)?)
        } else if self.public_build {
            None
        } else {
            // Dev: loopback, respeitando `MARVYR_PORT` como sempre foi.
            Some(ServerTarget {
                host: String::from("127.0.0.1"),
                port: env_port()?,
            })
        };
        let auth_url = self
            .cli_auth
            .clone()
            .or(self.env_auth.clone())
            .or(file_auth)
            .or(self.baked_auth.clone())
            .map(|url| url.trim().trim_end_matches('/').to_owned())
            .filter(|url| !url.is_empty());
        Ok(LaunchConfig { server, auth_url })
    }
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| String::from("MARVYR_PORT precisa ser um inteiro de 1 a 65535"))
}

/// `host` ou `host:porta`. IPv6 fica de fora: o socket local é IPv4.
fn parse_target(raw: &str) -> Result<ServerTarget, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(String::from("endereço de servidor vazio"));
    }
    if raw.starts_with('[') {
        return Err(format!(
            "IPv6 ainda não é suportado (\"{raw}\"); use IPv4 ou hostname"
        ));
    }
    match raw.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !host.ends_with(':') => Ok(ServerTarget {
            host: host.trim_matches(['[', ']']).to_owned(),
            port: parse_port(port)
                .map_err(|_| format!("porta inválida em \"{raw}\" (esperado host:porta)"))?,
        }),
        _ => Ok(ServerTarget {
            host: raw.to_owned(),
            port: DEFAULT_PORT,
        }),
    }
}

/// Lê `chave = "valor"` de um TOML plano. ponytail: só strings de primeiro
/// nível — o arquivo tem duas chaves; parser TOML completo quando crescer.
fn toml_value(file: &str, key: &str) -> Option<String> {
    file.lines().find_map(|line| {
        let line = line.split('#').next()?.trim();
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
    })
}

pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// Raiz de assets: `<pasta do executável>/assets` numa build distribuída;
/// em dev (cargo run), cai para o `assets/` do workspace.
pub fn asset_root() -> PathBuf {
    if let Some(dir) = exe_dir() {
        let beside = dir.join("assets");
        if beside.join("marvyr").is_dir() {
            return beside;
        }
        // macOS: Marvyr.app/Contents/MacOS/Marvyr lê de Contents/Resources,
        // onde o selo da assinatura cobre os arquivos sem xattrs (que o zip perde).
        let bundled = dir.join("..").join("Resources").join("assets");
        if bundled.join("marvyr").is_dir() {
            return bundled;
        }
    }
    if !PUBLIC_BUILD {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("assets"));
        if let Some(workspace) = workspace.filter(|path| path.is_dir()) {
            return workspace;
        }
    }
    // Build pública sem assets ao lado: o erro de asset aponta a pasta certa.
    exe_dir()
        .map(|dir| dir.join("assets"))
        .unwrap_or_else(|| PathBuf::from("assets"))
}

/// Pasta de dados do jogador (sessão, identidade de dev). Nunca ao lado do
/// `.exe` — a pasta do jogo pode ser somente leitura.
pub fn data_dir() -> PathBuf {
    let env = |name| std::env::var_os(name).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|home| home.join("Library").join("Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|home| home.join(".local").join("share")))
    };
    base.map(|dir| dir.join("Marvyr"))
        .unwrap_or_else(|| PathBuf::from("marvyr-data"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(host: &str, port: u16) -> Option<ServerTarget> {
        Some(ServerTarget {
            host: host.to_owned(),
            port,
        })
    }

    #[test]
    fn cli_beats_env_beats_file_beats_baked() {
        let mut sources = Sources {
            cli_server: Some("cli.marvyr.game:7000".into()),
            env_host: Some("env.marvyr.game".into()),
            env_port: Some("6000".into()),
            file: Some("server = \"file.marvyr.game:5500\"\nauth_url = \"https://a.b/\"".into()),
            baked_server: Some("baked.marvyr.game:5000".into()),
            ..Sources::default()
        };
        assert_eq!(
            sources.resolve().unwrap().server,
            target("cli.marvyr.game", 7000)
        );
        sources.cli_server = None;
        assert_eq!(
            sources.resolve().unwrap().server,
            target("env.marvyr.game", 6000)
        );
        sources.env_host = None;
        assert_eq!(
            sources.resolve().unwrap().server,
            target("file.marvyr.game", 5500)
        );
        assert_eq!(
            sources.resolve().unwrap().auth_url.as_deref(),
            Some("https://a.b")
        );
        sources.file = None;
        assert_eq!(
            sources.resolve().unwrap().server,
            target("baked.marvyr.game", 5000)
        );
    }

    #[test]
    fn public_build_never_falls_back_to_localhost() {
        let dev = Sources::default().resolve().unwrap();
        assert_eq!(dev.server, target("127.0.0.1", DEFAULT_PORT));
        let public = Sources {
            public_build: true,
            ..Sources::default()
        };
        assert_eq!(public.resolve().unwrap().server, None);
    }

    #[test]
    fn targets_accept_ip_hostname_and_default_port() {
        assert_eq!(parse_target("74.1.2.3:5001").unwrap().port, 5001);
        assert_eq!(
            parse_target("play.marvyr.game").unwrap(),
            ServerTarget {
                host: "play.marvyr.game".into(),
                port: DEFAULT_PORT
            }
        );
        assert!(parse_target("[::1]:5000").is_err(), "IPv6 recusado cedo");
        assert!(parse_target("host:abc").is_err());
        assert!(parse_target("").is_err());
        assert!(Sources {
            env_port: Some("0".into()),
            ..Sources::default()
        }
        .resolve()
        .is_err());
        // Porta de env inválida não derruba um `--server` que já tem porta.
        assert!(Sources {
            cli_server: Some("play.marvyr.game:5000".into()),
            env_port: Some("abc".into()),
            ..Sources::default()
        }
        .resolve()
        .is_ok());
    }

    #[test]
    fn localhost_resolves_to_ipv4() {
        let addr = ServerTarget {
            host: "localhost".into(),
            port: 5000,
        }
        .resolve()
        .unwrap();
        assert!(addr.is_ipv4());
    }

    #[test]
    fn dev_assets_resolve_to_workspace() {
        assert!(asset_root().join("marvyr").is_dir());
    }
}
