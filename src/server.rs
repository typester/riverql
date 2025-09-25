use crate::{
    ListenTarget,
    gql::{self, AppSchema, QueryRoot, SubscriptionRoot},
    river,
};
use anyhow::{Result, anyhow};
use async_graphql::{EmptyMutation, Schema};
use async_graphql_axum::{GraphQL, GraphQLSubscription};
use axum::{
    Router,
    extract::State,
    http::{self, header},
    response::Html,
    routing::{get, get_service},
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

#[cfg(unix)]
use std::fs;

pub async fn run(listen: ListenTarget) -> Result<()> {
    let (tx, _rx) = broadcast::channel::<river::Event>(1024);
    let river_state = gql::new_river_state();
    let schema: AppSchema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot)
        .data(tx.clone())
        .data(river_state.clone())
        .finish();

    info!("connecting to river status stream");
    let mut river_rx = river::RiverStatus::subscribe().map_err(|e| anyhow!(e.to_string()))?;
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
                if let Err(e) = fs::remove_file(&path) {
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
            http::HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        schema.sdl(),
    )
}
