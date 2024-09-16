mod errors;
mod extractors;
mod handlers;
mod middlewares;

use std::time::Duration;

use axum::{routing::get, Router};
use config::{Config, ConfigError};
use middlewares::timing_middleware;
use serde::Deserialize;
use tokio::{net::TcpListener, signal};
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Deserialize)]
struct AppConfig {
    host: String,
    port: usize,
}

fn load_config() -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        .set_default("host", "127.0.0.1")?
        .set_default("port", "3000")?
        .add_source(config::Environment::default())
        .build()?;
    config.try_deserialize::<AppConfig>()
}

fn setup_tracing() {
    let layer = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        format!(
            "{}=debug,tower_http=debug,axum=trace",
            env!("CARGO_CRATE_NAME")
        )
        .into()
    });
    tracing_subscriber::registry()
        .with(layer)
        .with(tracing_subscriber::fmt::layer().without_time())
        .init();
}

#[tokio::main]
async fn main() {
    let config = load_config().unwrap();
    setup_tracing();

    let app = Router::new()
        .route("/*url", get(handlers::handle_extract))
        .route("/_healthz", get(handlers::handle_health))
        .layer(axum::middleware::from_fn(timing_middleware))
        .layer((
            TraceLayer::new_for_http(),
            TimeoutLayer::new(Duration::from_secs(10)),
        ));

    let listener = TcpListener::bind((config.host, config.port as u16))
        .await
        .unwrap();

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
