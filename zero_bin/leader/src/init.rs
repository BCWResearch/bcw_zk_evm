use tracing_subscriber::{prelude::*, util::SubscriberInitExt, EnvFilter};
pub fn tracing() {
    tracing_subscriber::Registry::default()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .compact()
                .with_filter(EnvFilter::from_default_env()),
        )
        .init();
}

use std::io;

use dotenvy::dotenv;
use tracing::warn;

pub const EVM_ARITH_VER_KEY: &str = "EVM_ARITHMETIZATION_PKG_VER";

/// Attempt to load in the local `.env` if present and set any environment
/// variables specified inside of it.
///
/// To keep things simple, any IO error we will treat as the file not existing
/// and continue moving on without the `env` variables set.
pub fn load_dotenvy_vars_if_present() {
    match dotenv() {
        Ok(_) | Err(dotenvy::Error::Io(io::Error { .. })) => (),
        Err(e) => warn!("Found local `.env` file but was unable to parse it! (err: {e})",),
    }
}
