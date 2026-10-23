pub mod adapter;
pub mod model;

pub use adapter::{LoadReport, PgAdapter};
pub use model::Dataset;

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
    #[error("load requires empty accounts, services and transactions tables")]
    TablesNotEmpty,
}
