//! Rows of `accounts`, `services` and `transactions`, in the shape of `scripts/data.json`.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum BonusState {
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum PaymentTerm {
    Hour,
    Week,
    Month,
    QuarterYear,
    HalfYear,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ServiceStatus {
    Active,
    Blocked,
    Deleted,
    Suspended,
    ProlongWait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TransactionStatus {
    Performed,
    NotPerformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TransactionType {
    Order,
    Prolong,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub balance: i64,
    pub withdraw_balance: i64,
    pub total_replenished: i64,
    pub bonus_balance: i64,
    pub bonus_state: BonusState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Service {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
    pub payment_term: PaymentTerm,
    pub auto_prolong: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: ServiceStatus,
    pub price: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transaction {
    pub id: i64,
    pub amount: i32,
    pub status: TransactionStatus,
    pub performed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[serde(rename = "type")]
    pub kind: TransactionType,
    pub service_id: i64,
}

/// Everything `scripts/generate.py` writes to `data.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub services: Vec<Service>,
    #[serde(default)]
    pub transactions: Vec<Transaction>,
}

impl Dataset {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let raw = std::fs::read(path)?;
        Ok(serde_json::from_slice(&raw)?)
    }
}

/// One row of `public.client_features(as_of, horizon)`; see migration 007 for definitions.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[sqlx(rename_all = "camelCase")]
pub struct ClientFeatures {
    pub account_id: i64,
    pub email: String,
    pub services_count: i64,
    pub monthly_spend: f64,
    pub short_term_share: Option<f64>,
    pub no_auto_prolong_share: Option<f64>,
    pub prolong_wait_share: Option<f64>,
    pub failed_prolong_share: Option<f64>,
    pub expiring_revenue_share: Option<f64>,
    pub upcoming_payments: i64,
    pub spendable_balance: i64,
    pub payment_coverage: Option<f64>,
}
