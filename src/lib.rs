//! Online behavioral history for Aéza customer signals.
//!
//! The model evaluates an observation against the profile accumulated before
//! that observation, then updates the profile. It reports behavioral novelty
//! and deviation; it does not interpret either as churn on its own.

pub mod adapter;
pub mod analysis;
pub mod historical;
pub mod market;
pub mod market_store;
pub mod model;
pub mod sequences;
pub mod stats;

pub use adapter::{LoadReport, PgAdapter};
pub use model::{ClientFeatures, Dataset};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("failed to read dataset: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid dataset JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid dataset: {0}")]
    InvalidDataset(String),
    #[error("load requires empty accounts, services and transactions tables")]
    TablesNotEmpty,
    #[error("rollback below version 104 could delete existing client or market data")]
    UnsafeRollback,
}
