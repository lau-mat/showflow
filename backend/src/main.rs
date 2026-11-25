use axum::{routing::get, Router};
use sqlx::PgPool;
use std::env;

mod api_error;
mod auth;
mod caller_pass;
mod cors;
mod mail;
mod sharing;
mod shows;
mod sync_stream;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load local .env file if present (useful for development)
    let _ = dotenvy::dotenv();

    let auth = auth::AuthConfig::from_env()?;

    // 1. Read the connection string from environment variables
    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL environment variable must be set");

    println!("Connecting to PostgreSQL...");

    // 2. Create the connection pool
    let pool = PgPool::connect(&database_url).await?;

    // *Verification:* Check if the database connection was successfully established
    println!("Successfully connected to the database!");

    // 3. Automatically run any pending migrations from the 'migrations' folder
    println!("Running database migrations...");
    MIGRATOR.run(&pool).await?;

    // *Verification:* Check if migrations applied without error
    println!("Database schema is up to date and ready.");

    // Expired sessions and tokens are harmless (they never validate) but should not pile up.
    let housekeeping_pool = pool.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(error) = auth::purge_expired(&housekeeping_pool).await {
                eprintln!("Housekeeping failed: {error}");
            }
        }
    });

    start_server(pool, auth).await?;
    Ok(())
}

async fn start_server(pool: PgPool, auth: auth::AuthConfig) -> Result<(), std::io::Error> {
    // One database listener per instance feeds every connected live stream.
    let state = auth::state(pool.clone(), auth);
    sync_stream::spawn_listener(pool, state.hub.clone());
    let origins = cors::parse_origins(env::var("AUTH_ALLOWED_ORIGINS").ok().as_deref());
    let app = Router::new()
        .route("/", get(|| async { "Hello from Showflow Rust Backend!" }))
        .merge(auth::routes())
        .merge(shows::routes())
        .merge(sharing::routes())
        .merge(caller_pass::routes())
        .merge(sync_stream::routes())
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(origins, cors::layer));

    let address = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Backend server running on http://{address}");

    axum::serve(listener, app).await
}

#[cfg(test)]
mod tests {
    use super::MIGRATOR;

    #[test]
    fn embeds_initial_schema_migration() {
        assert!(MIGRATOR.iter().any(|migration| {
            migration.version == 202611112320 && migration.description == "initial schema"
        }));
    }
}
