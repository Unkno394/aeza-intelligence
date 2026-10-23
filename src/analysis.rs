//! Compares client risk vectors: robust per-feature deviation (median/MAD), Mahalanobis
//! distance, k nearest neighbours, deviation from similar clients and feature correlations.
//!
//! Every risk feature is oriented so that a positive deviation means "riskier than typical":
//! low payment coverage scores high, an unusually large balance scores negative.

use nalgebra::DMatrix;
use serde::Serialize;

use crate::model::ClientFeatures;
use crate::stats::{median, nearest, pearson, robust_scale};

struct Feature {
    name: &'static str,
    /// +1 when larger values mean more risk, -1 when smaller do.
    direction: f64,
    value: fn(&ClientFeatures) -> Option<f64>,
}

const RISK_FEATURES: [Feature; 5] = [
    Feature {
        name: "noAutoProlongShare",
        direction: 1.0,
        value: |c| c.no_auto_prolong_share,
    },
    Feature {
        name: "prolongWaitShare",
        direction: 1.0,
        value: |c| c.prolong_wait_share,
    },
    Feature {
        name: "failedProlongShare",
        direction: 1.0,
        value: |c| c.failed_prolong_share,
    },
    Feature {
        name: "expiringRevenueShare",
        direction: 1.0,
        value: |c| c.expiring_revenue_share,
    },
    Feature {
        name: "paymentCoverage",
        direction: -1.0,
        value: |c| Some(c.payment_coverage),
    },
];

/// What makes clients comparable: how many services, how much they spend
/// (log scale, spend spans orders of magnitude) and how short their payment terms are.
const CONTEXT_FEATURES: [Feature; 3] = [
    Feature {
        name: "servicesCount",
        direction: 1.0,
        value: |c| Some(c.services_count as f64),
    },
    Feature {
        name: "logMonthlySpend",
        direction: 1.0,
        value: |c| Some(c.monthly_spend.ln_1p()),
    },
    Feature {
        name: "shortTermShare",
        direction: 1.0,
        value: |c| c.short_term_share,
    },
];

#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// Neighbours for k-NN and for the peer group of conditional deviations.
    pub k: usize,
    /// Covariance shrinkage towards a scaled identity, 0..=1. Needed because with few
    /// clients (or strongly dependent features) the sample covariance is unstable or singular.
    pub shrinkage: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub features: Vec<&'static str>,
    pub centers: Vec<Center>,
    pub clients: Vec<ClientReport>,
    /// Pearson correlation of raw (imputed) risk features, in `features` order;
    /// `None` when a feature is constant.
    pub correlation: Vec<Vec<Option<f64>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Center {
    pub feature: &'static str,
    pub median: f64,
    /// 1.4826 · MAD (see [`robust_scale`]); 0 when every client has the same value.
    pub scale: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientReport {
    pub account_id: i64,
    pub email: String,
    /// Raw feature values; `None` was replaced by the median for the computations below.
    pub values: Vec<Option<f64>>,
    /// (value − median) / scale, oriented: positive = riskier than the typical client.
    pub robust_z: Vec<f64>,
    /// Distance of the whole oriented vector from the median client, accounting for
    /// feature covariance. Undirected: unusual in any direction counts.
    pub mahalanobis: Option<f64>,
    /// Most similar clients by risk vector and the mean distance to them; a large distance
    /// means even the closest clients look different.
    pub neighbours: Vec<i64>,
    pub neighbour_distance: Option<f64>,
    /// Clients most similar by context (services count, spend, payment terms).
    pub peers: Vec<i64>,
    /// robust_z minus the peers' median robust_z: deviation from comparable clients.
    pub conditional_z: Vec<f64>,
}

pub fn analyze(clients: &[ClientFeatures], params: Params) -> Report {
    let features = RISK_FEATURES.iter().map(|f| f.name).collect();
    if clients.is_empty() {
        return Report {
            features,
            centers: vec![],
            clients: vec![],
            correlation: vec![],
        };
    }
    let k = params.k.min(clients.len() - 1);

    let raw = columns(clients, &RISK_FEATURES);
    let imputed: Vec<Vec<f64>> = raw.iter().map(|c| impute(c)).collect();
    let (centers, z_columns) = standardize(&imputed, &RISK_FEATURES);
    let z_rows = transpose(&z_columns);

    let context = columns(clients, &CONTEXT_FEATURES);
    let (_, context_columns) = standardize(
        &context.iter().map(|c| impute(c)).collect::<Vec<_>>(),
        &CONTEXT_FEATURES,
    );
    let context_rows = transpose(&context_columns);

    let mahalanobis = mahalanobis(&z_rows, params.shrinkage);

    let reports = clients
        .iter()
        .enumerate()
        .map(|(i, client)| {
            let neighbours = nearest(&z_rows, i, k);
            let peers = nearest(&context_rows, i, k);
            let conditional_z = if peers.is_empty() {
                vec![0.0; RISK_FEATURES.len()]
            } else {
                (0..RISK_FEATURES.len())
                    .map(|j| {
                        let peer_z: Vec<f64> = peers.iter().map(|(p, _)| z_rows[*p][j]).collect();
                        z_rows[i][j] - median(&peer_z)
                    })
                    .collect()
            };
            ClientReport {
                account_id: client.account_id,
                email: client.email.clone(),
                values: raw.iter().map(|column| column[i]).collect(),
                robust_z: z_rows[i].clone(),
                mahalanobis: mahalanobis.as_ref().map(|d| d[i]),
                neighbour_distance: (!neighbours.is_empty()).then(|| {
                    neighbours.iter().map(|(_, d)| d).sum::<f64>() / neighbours.len() as f64
                }),
                neighbours: neighbours
                    .iter()
                    .map(|(n, _)| clients[*n].account_id)
                    .collect(),
                peers: peers.iter().map(|(p, _)| clients[*p].account_id).collect(),
                conditional_z,
            }
        })
        .collect();

    let correlation = imputed
        .iter()
        .map(|a| imputed.iter().map(|b| pearson(a, b)).collect())
        .collect();

    Report {
        features,
        centers,
        clients: reports,
        correlation,
    }
}

/// Feature values per column.
fn columns(clients: &[ClientFeatures], features: &[Feature]) -> Vec<Vec<Option<f64>>> {
    features
        .iter()
        .map(|f| clients.iter().map(f.value).collect())
        .collect()
}

/// Replaces missing values with the median of the present ones (0 if none is present).
fn impute(column: &[Option<f64>]) -> Vec<f64> {
    let present: Vec<f64> = column.iter().flatten().copied().collect();
    let fill = if present.is_empty() {
        0.0
    } else {
        median(&present)
    };
    column.iter().map(|v| v.unwrap_or(fill)).collect()
}

/// Oriented robust z-scores per column; a constant column scores 0 everywhere.
fn standardize(columns: &[Vec<f64>], features: &[Feature]) -> (Vec<Center>, Vec<Vec<f64>>) {
    columns
        .iter()
        .zip(features)
        .map(|(column, feature)| {
            let center = median(column);
            let scale = robust_scale(column, center);
            let z = column
                .iter()
                // `+ 0.0` turns the -0.0 produced by direction = -1 into 0.0.
                .map(|v| {
                    if scale > 0.0 {
                        feature.direction * (v - center) / scale + 0.0
                    } else {
                        0.0
                    }
                })
                .collect();
            (
                Center {
                    feature: feature.name,
                    median: center,
                    scale,
                },
                z,
            )
        })
        .unzip()
}

fn transpose(columns: &[Vec<f64>]) -> Vec<Vec<f64>> {
    (0..columns[0].len())
        .map(|i| columns.iter().map(|c| c[i]).collect())
        .collect()
}

/// Mahalanobis distance of each row from the origin, i.e. the median client, since rows
/// are robust z-scores. The covariance Σ = (1 − λ)·S + λ·(tr S / p)·I is shrunk towards a
/// scaled identity. `None` when it can't be inverted (fewer than two clients, or every
/// feature constant).
fn mahalanobis(rows: &[Vec<f64>], shrinkage: f64) -> Option<Vec<f64>> {
    let (n, p) = (rows.len(), rows[0].len());
    if n < 2 {
        return None;
    }
    let data = DMatrix::from_fn(n, p, |i, j| rows[i][j]);
    let means = data.row_mean();
    let centered = DMatrix::from_fn(n, p, |i, j| data[(i, j)] - means[j]);
    let sample = centered.transpose() * &centered / (n - 1) as f64;
    let target = DMatrix::identity(p, p) * (sample.trace() / p as f64);
    let lambda = shrinkage.clamp(0.0, 1.0);
    let covariance = sample * (1.0 - lambda) + target * lambda;
    let inverse = covariance.try_inverse()?;
    Some(
        rows.iter()
            .map(|row| {
                let x = DMatrix::from_row_slice(1, p, row);
                (&x * &inverse * x.transpose())[(0, 0)].max(0.0).sqrt()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: i64, coverage: f64, prolong_wait: f64) -> ClientFeatures {
        ClientFeatures {
            account_id: id,
            email: format!("user-{id}@example.test"),
            services_count: 3,
            monthly_spend: 1000.0,
            short_term_share: Some(0.3),
            no_auto_prolong_share: Some(0.3),
            prolong_wait_share: Some(prolong_wait),
            failed_prolong_share: None,
            expiring_revenue_share: Some(0.1),
            upcoming_payments: 100,
            spendable_balance: (coverage * 100.0) as i64,
            payment_coverage: coverage,
        }
    }

    fn analyze_default(clients: &[ClientFeatures]) -> Report {
        analyze(
            clients,
            Params {
                k: 2,
                shrinkage: 0.3,
            },
        )
    }

    #[test]
    fn low_coverage_is_risky_high_coverage_is_not() {
        let clients: Vec<_> = [1.0, 2.0, 3.0, 4.0, 5.0, 0.1, 10.0]
            .iter()
            .enumerate()
            .map(|(i, c)| client(i as i64 + 1, *c, 0.0))
            .collect();
        let report = analyze_default(&clients);
        let coverage_z = |id: i64| report.clients[id as usize - 1].robust_z[4];
        assert!(coverage_z(6) > 0.0, "coverage 0.1 must score as risky");
        assert!(coverage_z(7) < 0.0, "coverage 10 must score as safe");
    }

    #[test]
    fn outlier_has_largest_mahalanobis_distance() {
        let mut clients: Vec<_> = (1..=8)
            .map(|i| client(i, 2.0 + i as f64 * 0.1, 0.1 * (i % 3) as f64))
            .collect();
        clients.push(client(9, 0.05, 1.0));
        let report = analyze_default(&clients);
        let distances: Vec<f64> = report
            .clients
            .iter()
            .map(|c| c.mahalanobis.unwrap())
            .collect();
        let max = distances.iter().cloned().fold(f64::MIN, f64::max);
        assert_eq!(distances[8], max);
    }

    #[test]
    fn missing_values_are_imputed_and_constant_features_score_zero() {
        let clients: Vec<_> = (1..=4).map(|i| client(i, i as f64, 0.0)).collect();
        let report = analyze_default(&clients);
        for c in &report.clients {
            assert_eq!(c.values[2], None);
            assert_eq!(c.robust_z[2], 0.0);
            assert_eq!(c.neighbours.len(), 2);
        }
        assert_eq!(report.correlation[2][4], None);
    }

    #[test]
    fn single_client_has_no_neighbours_or_distance() {
        let report = analyze_default(&[client(1, 1.0, 0.0)]);
        let c = &report.clients[0];
        assert!(c.neighbours.is_empty() && c.peers.is_empty());
        assert_eq!(c.mahalanobis, None);
        assert_eq!(c.neighbour_distance, None);
    }
}
