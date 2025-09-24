mod gql;
mod river;

use async_graphql::{EmptyMutation, Schema};
use async_graphql_axum::{GraphQL, GraphQLSubscription};
use axum::{
    Router,
    response::Html,
    routing::{get, get_service},
};
use gql::{AppSchema, QueryRoot, SubscriptionRoot};
use tokio::sync::broadcast;

async fn graphiql() -> Html<String> {
    let html = async_graphql::http::GraphiQLSource::build()
        .endpoint("/graphql")
        .subscription_endpoint("/graphql")
        .finish();
    Html(html)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, _rx) = broadcast::channel::<river::Event>(1024);
    let schema: AppSchema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot)
        .data(tx.clone())
        .finish();

    let mut river_rx = river::RiverStatus::subscribe()?;
    tokio::spawn(async move {
        while let Some(ev) = river_rx.recv().await {
            let _ = tx.send(ev);
        }
    });

    let app = Router::new()
        .route("/graphiql", get(graphiql))
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
