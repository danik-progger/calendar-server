use std::{env, io};

use calendar_server::{app::App, db::PgEvents, http};
use tower::Layer;
use vercel_runtime::{Error, axum::VercelLayer};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let db_url = env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::other("Set DATABASE_URL"))?;
    let router = App::new(PgEvents::default())
        .prepare(http::router()?, &db_url)
        .await?;
    let router = VercelLayer::new().layer(router);
    vercel_runtime::run(router).await
}
