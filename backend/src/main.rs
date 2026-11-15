use axum::{routing::get, Router};
use sqlx::PgPool;
use std::env;

mod auth;

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

    start_server(pool, auth).await?;
    Ok(())
}

async fn start_server(pool: PgPool, auth: auth::AuthConfig) -> Result<(), std::io::Error> {
    let app = Router::new()
        .route("/", get(|| async { "Hello from Showflow Rust Backend!" }))
        .merge(auth::router(pool, auth));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    println!("Backend server running on http://localhost:8080");

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
