use std::path::PathBuf;

use aeza_intelligence::{Dataset, PgAdapter};
use anyhow::Context;
use clap::{Parser, Subcommand};

/// Manages the database schema and loads fixture data into PostgreSQL.
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
    /// Apply pending migrations from migrations/.
    Migrate,
    /// Revert applied migrations newer than --to (default: all of them).
    Rollback {
        #[arg(long, default_value_t = 0)]
        to: i64,
    },
    /// Load a data.json produced by scripts/generate.py into empty tables.
    Load {
        #[arg(default_value = "scripts/data.json")]
        path: PathBuf,
        /// Apply pending migrations before loading.
        #[arg(long)]
        migrate: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let adapter = PgAdapter::connect(&cli.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    match cli.command {
        Command::Migrate => {
            adapter.migrate().await?;
            println!("Migrations applied");
        }
        Command::Rollback { to } => {
            adapter.rollback(to).await?;
            println!("Rolled back to version {to}");
        }
        Command::Load { path, migrate } => {
            let data = Dataset::from_path(&path)
                .with_context(|| format!("failed to load {}", path.display()))?;
            if migrate {
                adapter.migrate().await?;
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
