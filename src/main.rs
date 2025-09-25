mod gql;
mod river;

use std::env;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Result, bail};
use argh::FromArgs;
use async_graphql::{EmptyMutation, Schema};
use async_graphql_axum::{GraphQL, GraphQLSubscription};
use axum::{
    Router,
    extract::State,
    http::{self, header},
    response::Html,
    routing::{get, get_service},
};
use futures_util::{SinkExt, StreamExt};
use gql::{AppSchema, QueryRoot, SubscriptionRoot};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;
use tokio_tungstenite::{
    WebSocketStream, client_async, connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};
use tracing::{debug, error, info, warn};

#[cfg(unix)]
use libc::geteuid;
use url::Url;

#[derive(Debug, Clone)]
enum ListenTarget {
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
enum EndpointTarget {
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
}

#[derive(Deserialize, Debug)]
struct ServerMsg {
    #[serde(rename = "type")]
    typ: String,
    #[serde(default)]
    payload: Option<Value>,
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
    } = argh::from_env();

    if server {
        if endpoint.is_some() || query.is_some() {
            bail!("--server does not take endpoint or query arguments");
        }
        let listen = parse_listen_addr(&listen)?;
        run_server(listen).await?
    } else {
        let endpoint_str = endpoint.unwrap_or_else(default_endpoint);
        let endpoint = parse_endpoint(&endpoint_str)?;
        run_subscriber(endpoint, query).await?
    };

    Ok(())
}

async fn run_server(listen: ListenTarget) -> Result<()> {
    let (tx, _rx) = broadcast::channel::<river::Event>(1024);
    let river_state = gql::new_river_state();
    let schema: AppSchema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot)
        .data(tx.clone())
        .data(river_state.clone())
        .finish();

    info!("connecting to river status stream");
    let mut river_rx =
        river::RiverStatus::subscribe().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    info!("river status stream connected");
    let tx_for_events = tx.clone();
    let state_for_events = river_state.clone();
    tokio::spawn(async move {
        while let Some(ev) = river_rx.recv().await {
            gql::update_river_state(&state_for_events, &ev);
            match tx_for_events.send(ev.clone()) {
                Ok(_) => debug!(?ev, "river event broadcasted"),
                Err(e) => warn!("failed to broadcast river event: {}", e),
            }
        }
    });

    let app = Router::new()
        .route("/graphiql", get(graphiql))
        .route("/schema", get(schema_sdl))
        .route(
            "/graphql",
            get_service(GraphQLSubscription::new(schema.clone()))
                .post_service(GraphQL::new(schema.clone())),
        )
        .with_state(schema);

    match listen {
        ListenTarget::Tcp(addr) => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            info!(protocol = "tcp", address = %addr, "server listening");
            axum::serve(listener, app).await?;
        }
        #[cfg(unix)]
        ListenTarget::Unix(path) => {
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    tokio::fs::create_dir_all(parent).await?;
                }
            }
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        return Err(e.into());
                    }
                }
            }
            let listener = tokio::net::UnixListener::bind(&path)?;
            info!(protocol = "unix", socket = %path.display(), "server listening");
            axum::serve(listener, app).await?;
        }
    }
    Ok(())
}

async fn run_subscriber(endpoint: EndpointTarget, query_arg: Option<String>) -> Result<()> {
    let query = match query_arg {
        Some(q) if q.starts_with('@') => fs::read_to_string(&q[1..])?,
        Some(q) => q,
        None => {
            let mut stdin = io::stdin();
            if stdin.is_terminal() {
                bail!("supply a GraphQL subscription or pipe one into stdin");
            }
            let mut s = String::new();
            stdin.read_to_string(&mut s)?;
            s
        }
    };

    match endpoint {
        EndpointTarget::Tcp(url) => {
            let mut req = url.clone().into_client_request()?;
            req.headers_mut().insert(
                header::SEC_WEBSOCKET_PROTOCOL,
                http::HeaderValue::from_static("graphql-transport-ws"),
            );

            let (mut ws, _resp) = match connect_async(req).await {
                Ok(v) => v,
                Err(e) => {
                    error!("connect error: {}", e);
                    bail!(
                        "websocket handshake failed; ensure server is at {url} and supports graphql-transport-ws"
                    );
                }
            };

            drive_subscription(&mut ws, &query).await?
        }
        #[cfg(unix)]
        EndpointTarget::Unix { socket, path } => {
            use tokio::net::UnixStream;

            let stream = match UnixStream::connect(&socket).await {
                Ok(s) => s,
                Err(e) => {
                    error!("unix connect error: {}", e);
                    return Err(e.into());
                }
            };

            let mut req = format!("ws://localhost{}", path).into_client_request()?;
            req.headers_mut().insert(
                header::SEC_WEBSOCKET_PROTOCOL,
                http::HeaderValue::from_static("graphql-transport-ws"),
            );

            let (mut ws, _resp) = match client_async(req, stream).await {
                Ok(v) => v,
                Err(e) => {
                    error!("connect error: {}", e);
                    bail!(
                        "websocket handshake failed; ensure unix socket {} accepts graphql-transport-ws",
                        socket.display()
                    );
                }
            };

            drive_subscription(&mut ws, &query).await?
        }
    }

    Ok(())
}

async fn drive_subscription<S>(ws: &mut WebSocketStream<S>, query: &str) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    ws.send(Message::Text(
        json!({
            "type": "connection_init",
            "payload": {}
        })
        .to_string(),
    ))
    .await?;

    loop {
        let Some(msg) = ws.next().await else {
            bail!("connection closed before ack");
        };
        let msg = msg?;
        if let Message::Text(txt) = msg {
            if let Ok(parsed) = serde_json::from_str::<ServerMsg>(&txt) {
                if parsed.typ == "connection_ack" {
                    break;
                }
            }
        }
    }

    let sub_id = "1";
    ws.send(Message::Text(
        json!({
            "id": sub_id,
            "type": "subscribe",
            "payload": { "query": query }
        })
        .to_string(),
    ))
    .await?;

    while let Some(msg) = ws.next().await {
        let m = msg?;
        match m {
            Message::Text(txt) => {
                if let Ok(parsed) = serde_json::from_str::<ServerMsg>(&txt) {
                    match parsed.typ.as_str() {
                        "next" => {
                            if let Some(payload) = parsed.payload {
                                println!("{}", payload);
                            }
                        }
                        "error" => {
                            error!(
                                "subscription error: {}",
                                parsed.payload.unwrap_or(serde_json::Value::Null)
                            );
                        }
                        "complete" => break,
                        _ => {}
                    }
                }
            }
            Message::Close(_) => break,
            _ => {
                warn!("unexpected websocket message: {:?}", m);
            }
        }
    }

    Ok(())
}

async fn graphiql() -> Html<String> {
    let html = async_graphql::http::GraphiQLSource::build()
        .endpoint("/graphql")
        .subscription_endpoint("/graphql")
        .finish();
    Html(html)
}

async fn schema_sdl(State(schema): State<gql::AppSchema>) -> impl axum::response::IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        schema.sdl(),
    )
}
