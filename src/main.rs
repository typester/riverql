mod client;
mod gql;
mod river;
mod server;

use std::env;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Result, bail};
use argh::FromArgs;

#[cfg(unix)]
use libc::geteuid;
use url::Url;

#[derive(Debug, Clone)]
pub enum ListenTarget {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

impl fmt::Display for ListenTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenTarget::Tcp(addr) => write!(f, "tcp://{}", addr),
            #[cfg(unix)]
            ListenTarget::Unix(path) => write!(f, "unix://{}", path.display()),
        }
    }
}

fn default_listen_addr() -> String {
    #[cfg(unix)]
    {
        if let Some(dir) = env::var_os("XDG_RUNTIME_DIR") {
            let mut path = PathBuf::from(dir);
            path.push("riverql.sock");
            return format!("unix://{}", path.display());
        }
        let uid = unsafe { geteuid() };
        return format!("unix:///run/user/{uid}/riverql.sock");
    }

    #[cfg(not(unix))]
    {
        "tcp://127.0.0.1:8080".to_string()
    }
}

fn default_endpoint() -> String {
    match parse_listen_addr(&default_listen_addr()) {
        Ok(ListenTarget::Tcp(addr)) => format!("ws://{addr}/graphql"),
        #[cfg(unix)]
        Ok(ListenTarget::Unix(path)) => format!("unix://{}#/graphql", path.display()),
        Err(_) => "ws://127.0.0.1:8080/graphql".to_string(),
    }
}

fn parse_listen_addr(value: &str) -> Result<ListenTarget> {
    #[cfg(unix)]
    if let Some(rest) = value.strip_prefix("unix://") {
        let path = PathBuf::from(rest);
        if path.as_os_str().is_empty() {
            bail!("unix listen path cannot be empty");
        }
        return Ok(ListenTarget::Unix(path));
    }

    if let Some(rest) = value.strip_prefix("tcp://") {
        let addr: SocketAddr = rest.parse()?;
        return Ok(ListenTarget::Tcp(addr));
    }

    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok(ListenTarget::Tcp(addr));
    }

    #[cfg(unix)]
    {
        if !value.is_empty() {
            return Ok(ListenTarget::Unix(PathBuf::from(value)));
        }
    }

    bail!("invalid listen address {value:?}");
}

#[derive(Debug, Clone)]
pub enum EndpointTarget {
    Tcp(Url),
    #[cfg(unix)]
    Unix {
        socket: PathBuf,
        path: String,
    },
}

fn normalize_graphql_path<S: AsRef<str>>(input: S) -> String {
    let p = input.as_ref();
    if p.is_empty() {
        "/graphql".to_string()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{}", p)
    }
}

fn parse_endpoint(value: &str) -> Result<EndpointTarget> {
    #[cfg(unix)]
    if let Some(rest) = value.strip_prefix("unix://") {
        let mut parts = rest.splitn(2, '#');
        let socket_part = parts.next().unwrap_or_default();
        if socket_part.is_empty() {
            bail!("unix endpoint must include socket path");
        }
        let socket = PathBuf::from(socket_part);
        let path_part = parts.next().unwrap_or("/graphql");
        let path = normalize_graphql_path(path_part);
        return Ok(EndpointTarget::Unix { socket, path });
    }

    let mut candidate = if value.starts_with("ws://") || value.starts_with("wss://") {
        Url::parse(value)?
    } else if let Some(rest) = value.strip_prefix("tcp://") {
        Url::parse(&format!("ws://{}", rest))?
    } else if let Some(rest) = value.strip_prefix("http://") {
        Url::parse(&format!("ws://{}", rest))?
    } else if let Some(rest) = value.strip_prefix("https://") {
        Url::parse(&format!("wss://{}", rest))?
    } else if value.contains("//") {
        bail!("unsupported endpoint scheme in {value:?}");
    } else {
        Url::parse(&format!("ws://{}", value))?
    };

    if candidate.path() == "/" || candidate.path().is_empty() {
        candidate.set_path("/graphql");
    }

    Ok(EndpointTarget::Tcp(candidate))
}

#[derive(FromArgs, Debug)]
/// RiverQL CLI combining GraphQL server and subscription client.
struct Cli {
    /// run the GraphQL server (default runs subscription client)
    #[argh(switch)]
    server: bool,

    /// listen address (tcp://host:port or unix://path)
    #[argh(option, default = "default_listen_addr()")]
    listen: String,

    /// websocket endpoint for subscriptions (e.g. ws://host:port/graphql or unix://path#/graphql)
    #[argh(option)]
    endpoint: Option<String>,

    /// inline query or @file for subscription mode; defaults to stdin when omitted
    #[argh(positional)]
    query: Option<String>,

    /// show version information
    #[argh(switch)]
    version: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "riverql=info,tower_http=info".into()),
        )
        .with_target(false)
        .compact()
        .init();

    let Cli {
        server,
        listen,
        endpoint,
        query,
        version,
    } = argh::from_env();

    if version {
        println!("riverql {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if server {
        if endpoint.is_some() || query.is_some() {
            bail!("--server does not take endpoint or query arguments");
        }
        let listen = parse_listen_addr(&listen)?;
        server::run(listen).await?
    } else {
        let endpoint_value = endpoint.unwrap_or_else(default_endpoint);
        let endpoint = parse_endpoint(&endpoint_value)?;
        client::run(endpoint, query).await?
    };

    Ok(())
}
