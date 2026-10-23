use std::path::PathBuf;

use aeza_intelligence::{Dataset, PgAdapter};
use anyhow::Context;
use clap::{Parser, Subcommand};

/// Loads fixture data into PostgreSQL.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// e.g. postgres://user:password@localhost:5432/db
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create enum types, tables and indexes (idempotent).
    Schema,
    /// Load a data.json produced by scripts/generate.py into empty tables.
    Load {
        #[arg(default_value = "scripts/data.json")]
        path: PathBuf,
        /// Apply the schema before loading.
        #[arg(long)]
        with_schema: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let adapter = PgAdapter::connect(&cli.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    match cli.command {
        Command::Schema => {
            adapter.apply_schema().await?;
            println!("Schema applied");
        }
        Command::Load { path, with_schema } => {
            let data = Dataset::from_path(&path)
                .with_context(|| format!("failed to load {}", path.display()))?;
            if with_schema {
                adapter.apply_schema().await?;
            }
            let report = adapter.load(&data).await?;
            println!(
                "Loaded {} accounts, {} services, {} transactions",
                report.accounts, report.services, report.transactions
            );
        }
    }
    Ok(())
}
