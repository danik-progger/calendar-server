use std::{env, io, net::SocketAddr};

use calendar_server::{app::App, auth::Auth, db::PgEvents, http};

fn main() -> io::Result<()> {
    if let Err(error) = dotenvy::from_path(".env")
        && !error.not_found()
    {
        return Err(io::Error::other("Could not load .env"));
    }
    let db_url = env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::other("Set DATABASE_URL"))?;
    let bind: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .ok()
        .ok_or_else(|| io::Error::other("Set BIND_ADDR to a valid IP address and port"))?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let db = PgEvents::default();
            let router = http::router(
                &Auth::new(
                    &env::var("AUTH_BASE_URL")
                        .map_err(|_| io::Error::other("Set AUTH_BASE_URL"))?,
                )
                .map_err(io::Error::other)?,
            )
            .map_err(io::Error::other)?;
            App::new(db)
                .run(router, &db_url, bind)
                .await
                .map_err(|error| match error.downcast::<io::Error>() {
                    Ok(error) => *error,
                    Err(error) => io::Error::other(error),
                })
        })
}
