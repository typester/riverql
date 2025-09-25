mod gql;
mod river;

use std::fs;
use std::io::{self, Read};

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
use tokio::sync::broadcast;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};

#[derive(FromArgs, Debug)]
/// RiverQL CLI combining daemon and subscription client.
struct Cli {
    /// run the GraphQL daemon (default runs subscription client)
    #[argh(switch)]
    daemon: bool,

    /// websocket endpoint for subscriptions
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
    let Cli {
        daemon,
        endpoint,
        query,
    } = argh::from_env();

    if daemon {
        if endpoint.is_some() || query.is_some() {
            bail!("--daemon does not take endpoint or query arguments");
        }
        run_daemon().await?
    } else {
        run_subscriber(endpoint, query).await?
    };

    Ok(())
}

async fn run_daemon() -> Result<()> {
    let (tx, _rx) = broadcast::channel::<river::Event>(1024);
    let schema: AppSchema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot)
        .data(tx.clone())
        .finish();

    let mut river_rx =
        river::RiverStatus::subscribe().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    tokio::spawn(async move {
        while let Some(ev) = river_rx.recv().await {
            let _ = tx.send(ev);
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

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 8080)).await?;
    println!("GraphiQL: http://127.0.0.1:8080/graphiql");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_subscriber(endpoint: Option<String>, query_arg: Option<String>) -> Result<()> {
    let endpoint = endpoint.unwrap_or_else(|| "ws://127.0.0.1:8080/graphql".to_string());

    let query = match query_arg {
        Some(q) if q.starts_with('@') => fs::read_to_string(&q[1..])?,
        Some(q) => q,
        None => {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s)?;
            s
        }
    };

    let mut req = (&endpoint).into_client_request()?;
    req.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        http::HeaderValue::from_static("graphql-transport-ws"),
    );

    let (mut ws, _resp) = match connect_async(req).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("connect error: {}", e);
            bail!(
                "websocket handshake failed; ensure server is at {endpoint} and supports graphql-transport-ws"
            );
        }
    };

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
                            eprintln!("error: {}", parsed.payload.unwrap_or(Value::Null));
                        }
                        "complete" => break,
                        _ => {}
                    }
                }
            }
            Message::Close(_) => break,
            _ => {
                eprintln!("unexpected message: {:?}", m);
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
