use crate::EndpointTarget;
use anyhow::{Result, bail};
use axum::http::{HeaderValue, header};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, IsTerminal, Read};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    WebSocketStream, client_async, connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};
use tracing::{error, warn};

#[derive(Deserialize, Debug)]
struct ServerMsg {
    #[serde(rename = "type")]
    typ: String,
    #[serde(default)]
    payload: Option<Value>,
}

pub async fn run(endpoint: EndpointTarget, query_arg: Option<String>) -> Result<()> {
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
                HeaderValue::from_static("graphql-transport-ws"),
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
                HeaderValue::from_static("graphql-transport-ws"),
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
