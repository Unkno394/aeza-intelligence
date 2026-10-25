use std::path::PathBuf;

use aeza_intelligence::analysis::{self, Params, Report};
use aeza_intelligence::{Dataset, PgAdapter, passwords};
use anyhow::Context;
use chrono::{DateTime, Utc};
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
    /// Revert applied migrations newer than --to (versions below 104 are protected).
    Rollback {
        #[arg(long, default_value_t = 104)]
        to: i64,
    },
    /// Load a data.json produced by scripts/given/generate.py into empty tables.
    Load {
        #[arg(default_value = "scripts/given/data.json")]
        path: PathBuf,
        /// Apply pending migrations before loading.
        #[arg(long)]
        migrate: bool,
        /// Exit successfully without loading when the client tables already have rows.
        #[arg(long)]
        skip_if_loaded: bool,
    },
    /// Compare client risk vectors (robust deviations, Mahalanobis, k-NN, peers, correlations).
    Analyze {
        /// Moment to evaluate at, RFC 3339 [default: now].
        #[arg(long)]
        as_of: Option<DateTime<Utc>>,
        /// How far ahead "soon" and "upcoming payments" look, as a PostgreSQL interval.
        #[arg(long, default_value = "7 days")]
        horizon: String,
        /// Neighbours for k-NN and the peer group.
        #[arg(long, default_value_t = 3)]
        k: usize,
        /// Covariance shrinkage for Mahalanobis, 0..=1.
        #[arg(long, default_value_t = 0.3)]
        shrinkage: f64,
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Set the DEFAULT_AUTH password of an existing public.admins row. The
    /// password is read from the first line of stdin.
    SetPassword { email: String },
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
        Command::SetPassword { email } => {
            let mut password = String::new();
            std::io::stdin().read_line(&mut password)?;
            let password = password.trim_end_matches(['\r', '\n']);
            anyhow::ensure!(
                password.chars().count() >= passwords::MIN_LENGTH,
                "the password must be at least {} characters",
                passwords::MIN_LENGTH
            );
            let updated = sqlx::query(
                "UPDATE public.admins SET password_hash = $2 WHERE lower(email) = lower($1)",
            )
            .bind(&email)
            .bind(passwords::hash(password))
            .execute(adapter.pool())
            .await?
            .rows_affected();
            anyhow::ensure!(updated == 1, "{email} is not in public.admins");
            println!("{email}: password set");
        }
        Command::Rollback { to } => {
            adapter.rollback(to).await?;
            println!("Rolled back to version {to}");
        }
        Command::Load {
            path,
            migrate,
            skip_if_loaded,
        } => {
            let data = Dataset::from_path(&path)
                .with_context(|| format!("failed to load {}", path.display()))?;
            if migrate {
                adapter.migrate().await?;
            }
            if skip_if_loaded && adapter.has_client_data().await? {
                println!("Client tables already contain data, skipping load");
                return Ok(());
            }
            let report = adapter.load(&data).await?;
            println!(
                "Loaded {} accounts, {} services, {} transactions",
                report.accounts, report.services, report.transactions
            );
        }
        Command::Analyze {
            as_of,
            horizon,
            k,
            shrinkage,
            json,
        } => {
            let as_of = as_of.unwrap_or_else(Utc::now);
            let clients = adapter.client_features(as_of, &horizon).await?;
            let report = analysis::analyze(&clients, Params { k, shrinkage });
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_report(&report, as_of, &horizon);
            }
        }
    }
    Ok(())
}

const SHORT_NAMES: [&str; 5] = ["noAP", "pWait", "failPr", "expRev", "cover"];

fn print_report(report: &Report, as_of: DateTime<Utc>, horizon: &str) {
    println!(
        "As of {as_of}, horizon {horizon}. Robust z > 0 means riskier than the median client."
    );
    println!("Features: noAP=noAutoProlongShare pWait=prolongWaitShare failPr=failedProlongShare");
    println!("          expRev=expiringRevenueShare cover=paymentCoverage\n");

    println!("{:<8}{:>10}{:>10}", "feature", "median", "scale");
    for (center, name) in report.centers.iter().zip(SHORT_NAMES) {
        println!("{name:<8}{:>10.3}{:>10.3}", center.median, center.scale);
    }

    let mut clients: Vec<_> = report.clients.iter().collect();
    clients.sort_by(|a, b| {
        b.mahalanobis
            .unwrap_or(0.0)
            .total_cmp(&a.mahalanobis.unwrap_or(0.0))
    });

    println!("\nRobust z by feature, sorted by Mahalanobis distance:");
    print!("{:<6}", "acct");
    for name in SHORT_NAMES {
        print!("{name:>8}");
    }
    println!(
        "{:>8}{:>8}  {:<12}{:<12}vs peers (max)",
        "maha", "kNN", "neighbours", "peers"
    );
    for c in clients {
        print!("{:<6}", c.account_id);
        for z in &c.robust_z {
            print!("{z:>8.2}");
        }
        let (worst, worst_z) = c
            .conditional_z
            .iter()
            .zip(SHORT_NAMES)
            .max_by(|a, b| a.0.total_cmp(b.0))
            .map(|(z, name)| (name, *z))
            .unwrap_or(("-", 0.0));
        println!(
            "{:>8}{:>8}  {:<12}{:<12}{worst} {worst_z:+.2}",
            fmt_opt(c.mahalanobis),
            fmt_opt(c.neighbour_distance),
            join_ids(&c.neighbours),
            join_ids(&c.peers),
        );
    }

    println!("\nPearson correlation (|r| >= 0.8 marked *: likely redundant):");
    print!("{:<8}", "");
    for name in SHORT_NAMES {
        print!("{name:>9}");
    }
    println!();
    for (i, (row, name)) in report.correlation.iter().zip(SHORT_NAMES).enumerate() {
        print!("{name:<8}");
        for (j, r) in row.iter().enumerate() {
            match r {
                Some(r) => print!(
                    "{r:>8.2}{}",
                    if i != j && r.abs() >= 0.8 { '*' } else { ' ' }
                ),
                None => print!("{:>8} ", "-"),
            }
        }
        println!();
    }
}

fn fmt_opt(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_owned(), |v| format!("{v:.2}"))
}

fn join_ids(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}
