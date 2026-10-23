//! Small statistics helpers over `f64` slices.

const EPS: f64 = 1e-12;

/// Median of a non-empty slice.
pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

/// Robust standard deviation around `center`: 1.4826 · MAD, which matches the standard
/// deviation for normal data. When more than half of the values coincide MAD is 0, so it
/// falls back to 1.2533 · mean absolute deviation; 0 means all values are equal.
pub fn robust_scale(values: &[f64], center: f64) -> f64 {
    let deviations: Vec<f64> = values.iter().map(|v| (v - center).abs()).collect();
    let mad = median(&deviations);
    if mad > EPS {
        return 1.4826 * mad;
    }
    let mean_ad = deviations.iter().sum::<f64>() / deviations.len() as f64;
    if mean_ad > EPS { 1.2533 * mean_ad } else { 0.0 }
}

/// Pearson correlation; `None` when either side has no variance.
pub fn pearson(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len() as f64;
    let (mean_a, mean_b) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut cov, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        cov += (x - mean_a) * (y - mean_b);
        var_a += (x - mean_a).powi(2);
        var_b += (y - mean_b).powi(2);
    }
    (var_a > EPS && var_b > EPS).then(|| cov / (var_a * var_b).sqrt())
}

pub fn euclidean(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// The `k` points closest to `points[target]`, excluding itself, nearest first.
pub fn nearest(points: &[Vec<f64>], target: usize, k: usize) -> Vec<(usize, f64)> {
    let mut distances: Vec<(usize, f64)> = points
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != target)
        .map(|(i, p)| (i, euclidean(&points[target], p)))
        .collect();
    distances.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    distances.truncate(k);
    distances
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd_and_even() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn robust_scale_ignores_outlier() {
        let values = [1.0, 2.0, 3.0, 4.0, 1000.0];
        // MAD = median(|x - 3|) = median(2, 1, 0, 1, 997) = 1
        assert!((robust_scale(&values, 3.0) - 1.4826).abs() < 1e-9);
    }

    #[test]
    fn robust_scale_falls_back_when_mad_is_zero() {
        let values = [0.0, 0.0, 0.0, 1.0];
        assert!((robust_scale(&values, 0.0) - 1.2533 * 0.25).abs() < 1e-9);
        assert_eq!(robust_scale(&[5.0, 5.0], 5.0), 0.0);
    }

    #[test]
    fn pearson_signs_and_constant() {
        assert!((pearson(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]).unwrap() - 1.0).abs() < 1e-9);
        assert!((pearson(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]).unwrap() + 1.0).abs() < 1e-9);
        assert_eq!(pearson(&[1.0, 2.0, 3.0], &[7.0, 7.0, 7.0]), None);
    }

    #[test]
    fn nearest_excludes_self_and_sorts() {
        let points = vec![vec![0.0], vec![5.0], vec![1.0], vec![2.0]];
        let found: Vec<usize> = nearest(&points, 0, 2).into_iter().map(|(i, _)| i).collect();
        assert_eq!(found, vec![2, 3]);
    }
}
