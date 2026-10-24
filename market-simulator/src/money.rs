//! All billing arithmetic uses integer nano-USD. Floating point is only an API/utility boundary.
pub const SCALE: i128 = 1_000_000_000;
pub fn ratio(s: &str) -> Result<(i128, i128), String> {
    if s.is_empty() || s.len() > 64 {
        return Err("invalid decimal".into());
    }
    let (mantissa, exp) = if let Some((a, b)) = s.split_once(['e', 'E']) {
        (a, b.parse::<i32>().map_err(|_| "decimal exponent")?)
    } else {
        (s, 0)
    };
    let mut n = 0i128;
    let mut places = 0i32;
    let mut point = false;
    for c in mantissa.chars() {
        if c == '.' && !point {
            point = true;
            continue;
        }
        let d = c.to_digit(10).ok_or("nonnegative decimal required")?;
        n = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(d as i128))
            .ok_or("decimal overflow")?;
        if point {
            places += 1;
        }
    }
    let power = places - exp;
    if power.abs() > 27 {
        return Err("decimal precision out of range".into());
    }
    if power >= 0 {
        Ok((n, 10i128.pow(power as u32)))
    } else {
        Ok((
            n.checked_mul(10i128.pow((-power) as u32))
                .ok_or("decimal overflow")?,
            1,
        ))
    }
}
pub fn round(n: i128, d: i128) -> Result<i128, String> {
    if n < 0 || d <= 0 {
        return Err("invalid monetary ratio".into());
    }
    let q = n / d;
    let r = n % d;
    Ok(q + i128::from(r > d - r || (r == d - r && q % 2 == 1)))
}
pub fn decimal(s: &str) -> Result<i128, String> {
    let (n, d) = ratio(s)?;
    round(n.checked_mul(SCALE).ok_or("money overflow")?, d)
}
pub fn ticks(n: f64) -> i128 {
    if n == 0. {
        return 0;
    }
    decimal(&n.to_string()).expect("validated finite monetary input")
}
pub fn apply(base: i128, multiplier: f64) -> i128 {
    let (n, d) = ratio(&multiplier.to_string()).expect("validated multiplier");
    round(base * n, d).expect("validated multiplier")
}
pub fn float(ticks: i128) -> f64 {
    ticks as f64 / SCALE as f64
}
pub fn text(ticks: i128) -> String {
    let sign = if ticks < 0 { "-" } else { "" };
    let n = ticks.abs();
    format!("{sign}{}.{:09}", n / SCALE, n % SCALE)
}
pub fn convert_to(
    amount: &str,
    currency: &str,
    target: &str,
    fx: &crate::market_data::Fx,
    unit: &str,
    hours: Option<u32>,
) -> Result<i128, String> {
    let (a, ad) = ratio(amount)?;
    let from = fx
        .rub_per_unit
        .get(currency)
        .ok_or("missing source currency rate")?;
    let (f, fd) = ratio(from)?;
    let (u, ud) = ratio(fx.rub_per_unit.get(target).ok_or("missing USD rate")?)?;
    let h = match unit {
        "month" => 1,
        "hour" => hours
            .filter(|h| *h > 0 && *h <= 744)
            .ok_or("hourly offer requires explicit 1..744 billing hours")?
            as i128,
        _ => return Err("unsupported billing unit".into()),
    };
    let n = a
        .checked_mul(f)
        .and_then(|v| v.checked_mul(ud))
        .and_then(|v| v.checked_mul(SCALE))
        .and_then(|v| v.checked_mul(h))
        .ok_or("FX overflow")?;
    let d = ad
        .checked_mul(fd)
        .and_then(|v| v.checked_mul(u))
        .ok_or("FX overflow")?;
    round(n, d)
}

pub fn convert(
    amount: &str,
    currency: &str,
    fx: &crate::market_data::Fx,
    unit: &str,
    hours: Option<u32>,
) -> Result<i128, String> {
    convert_to(amount, currency, "USD", fx, unit, hours)
}
