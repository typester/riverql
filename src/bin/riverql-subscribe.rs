use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{env, fs, io::{self, Read}};
use tokio_tungstenite::tungstenite::protocol::Message;
use axum::http::{self, Request, header};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Deserialize, Debug)]
struct ServerMsg {
    #[serde(rename = "type")]
    typ: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    payload: Option<Value>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();

    let mut endpoint = String::from("ws://127.0.0.1:8080/graphql");
    if let Some(pos) = args.iter().position(|a| a == "--endpoint") {
        if pos + 1 < args.len() {
            endpoint = args.remove(pos + 1);
            args.remove(pos);
        }
    }

    let query = if !args.is_empty() {
        let q = &args[0];
        if q.starts_with('@') {
            fs::read_to_string(&q[1..])?
        } else {
            q.clone()
        }
    } else {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        s
    };

    // WebSocket handshake with GraphQL subprotocol
    let mut req = (&endpoint).into_client_request()?;
    req.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        http::HeaderValue::from_static("graphql-transport-ws"),
    );
    let (mut ws, _resp) = match tokio_tungstenite::connect_async(req).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("connect error: {}", e);
            anyhow::bail!("websocket handshake failed; ensure server is at {endpoint} and supports graphql-transport-ws");
        }
    };

    // connection_init
    ws.send(Message::Text(json!({
        "type": "connection_init",
        "payload": {}
    }).to_string())).await?;

    // wait for connection_ack
    loop {
        let Some(msg) = ws.next().await else { anyhow::bail!("connection closed before ack") };
        let msg = msg?;
        if let Message::Text(txt) = msg {
            if let Ok(parsed) = serde_json::from_str::<ServerMsg>(&txt) {
                if parsed.typ == "connection_ack" { break; }
                // ignore keepalive etc.
            }
        }
    }

    // subscribe
    let sub_id = "1";
    ws.send(Message::Text(json!({
        "id": sub_id,
        "type": "subscribe",
        "payload": { "query": query }
    }).to_string())).await?;

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
