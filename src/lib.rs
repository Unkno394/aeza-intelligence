//! Online behavioral history for Aéza customer signals.
//!
//! The model evaluates an observation against the profile accumulated before
//! that observation, then updates the profile. It reports behavioral novelty
//! and deviation; it does not interpret either as churn on its own.

pub mod adapter;
pub mod historical;
pub mod market;
pub mod market_store;
pub mod model;

pub use adapter::{LoadReport, PgAdapter};
pub use model::Dataset;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("failed to read dataset: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid dataset JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("load requires empty accounts, services and transactions tables")]
    TablesNotEmpty,
}
