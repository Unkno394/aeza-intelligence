//! Rows of `accounts`, `services` and `transactions`, in the shape of `scripts/data.json`.

use std::collections::HashSet;
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

    pub fn validate(&self) -> Result<(), Error> {
        let mut account_ids = HashSet::new();
        let mut emails = HashSet::new();
        for account in &self.accounts {
            if !account_ids.insert(account.id) || !emails.insert(account.email.to_lowercase()) {
                return Err(Error::InvalidDataset(format!(
                    "duplicate account {}",
                    account.id
                )));
            }
            if [
                account.balance,
                account.withdraw_balance,
                account.total_replenished,
                account.bonus_balance,
            ]
            .iter()
            .any(|value| *value < 0)
            {
                return Err(Error::InvalidDataset(format!(
                    "negative account amount {}",
                    account.id
                )));
            }
        }
        let mut service_ids = HashSet::new();
        for service in &self.services {
            if !service_ids.insert(service.id)
                || !account_ids.contains(&service.owner_id)
                || service.price < 0
                || service.expires_at < service.created_at
            {
                return Err(Error::InvalidDataset(format!(
                    "invalid service {}",
                    service.id
                )));
            }
        }
        let mut transaction_ids = HashSet::new();
        for transaction in &self.transactions {
            if !transaction_ids.insert(transaction.id)
                || !service_ids.contains(&transaction.service_id)
                || transaction.amount < 0
                || (transaction.status == TransactionStatus::Performed)
                    != transaction.performed_at.is_some()
            {
                return Err(Error::InvalidDataset(format!(
                    "invalid transaction {}",
                    transaction.id
                )));
            }
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn fixture() -> Dataset {
        let created_at = Utc::now() - Duration::days(2);
        Dataset {
            accounts: vec![Account {
                id: 1,
                email: "client@example.test".into(),
                balance: 100,
                withdraw_balance: 50,
                total_replenished: 200,
                bonus_balance: 25,
                bonus_state: BonusState::Unlocked,
            }],
            services: vec![Service {
                id: 10,
                name: "vps-10".into(),
                owner_id: 1,
                payment_term: PaymentTerm::Month,
                auto_prolong: true,
                created_at,
                expires_at: created_at + Duration::days(30),
                status: ServiceStatus::Active,
                price: 100,
            }],
            transactions: vec![Transaction {
                id: 20,
                amount: 100,
                status: TransactionStatus::Performed,
                performed_at: Some(created_at),
                created_at,
                kind: TransactionType::Prolong,
                service_id: 10,
            }],
        }
    }

    #[test]
    fn complete_snapshot_with_references_and_money_units_is_valid() {
        let data = fixture();
        assert!(data.validate().is_ok());
        assert_eq!(data.accounts[0].balance, 100);
        assert_eq!(data.services[0].price, 100);
        assert_eq!(data.transactions[0].amount, 100);
    }

    #[test]
    fn validation_rejects_duplicate_accounts_case_insensitive_emails_and_duplicate_services() {
        let mut data = fixture();
        data.accounts.push(data.accounts[0].clone());
        assert!(data.validate().is_err());

        let mut data = fixture();
        let mut duplicate = data.accounts[0].clone();
        duplicate.id = 2;
        duplicate.email = "CLIENT@EXAMPLE.TEST".into();
        data.accounts.push(duplicate);
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.services.push(data.services[0].clone());
        assert!(data.validate().is_err());
    }

    #[test]
    fn validation_rejects_invalid_amounts_or_missing_parent_rows() {
        let mut data = fixture();
        data.accounts[0].balance = -1;
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.services[0].owner_id = 99;
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.services[0].price = -1;
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.transactions[0].service_id = 99;
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.transactions[0].amount = -1;
        assert!(data.validate().is_err());
    }

    #[test]
    fn validation_rejects_invalid_service_dates_and_transaction_outcomes() {
        let mut data = fixture();
        data.services[0].expires_at = data.services[0].created_at - Duration::seconds(1);
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.transactions[0].performed_at = None;
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.transactions[0].status = TransactionStatus::NotPerformed;
        assert!(data.validate().is_err());
    }

    #[test]
    fn empty_snapshot_is_valid_but_required_row_fields_and_unknown_statuses_are_not() {
        assert!(Dataset::default().validate().is_ok());

        let mut data = fixture();
        data.accounts[0].email.clear();
        // Empty email syntax is a source-quality issue, not a relationship
        // constraint: snapshot validation deliberately preserves source rows.
        assert!(data.validate().is_ok());

        for raw in [
            r#"{"accounts":[{"id":1}],"services":[],"transactions":[]}"#,
            r#"{"accounts":[{"id":"one","email":"x","balance":0,"withdrawBalance":0,"totalReplenished":0,"bonusBalance":0,"bonusState":"unlocked"}],"services":[],"transactions":[]}"#,
            r#"{"accounts":[{"id":1,"email":"x","balance":0,"withdrawBalance":0,"totalReplenished":0,"bonusBalance":0,"bonusState":"unknown"}],"services":[],"transactions":[]}"#,
            r#"{"accounts":[],"services":[{"id":1,"status":"running"}],"transactions":[]}"#,
        ] {
            assert!(serde_json::from_str::<Dataset>(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn validation_rejects_duplicate_transaction_ids_and_invalid_parent_status_pairs() {
        let mut data = fixture();
        data.transactions.push(data.transactions[0].clone());
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.transactions[0].status = TransactionStatus::Performed;
        data.transactions[0].performed_at = None;
        assert!(data.validate().is_err());

        let mut data = fixture();
        data.transactions[0].status = TransactionStatus::NotPerformed;
        data.transactions[0].performed_at = Some(data.transactions[0].created_at);
        assert!(data.validate().is_err());
    }
}
