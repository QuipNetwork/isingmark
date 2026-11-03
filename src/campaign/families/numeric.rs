// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact conversion between natural energy units in instance files and wire milli.
//!
//! People write energies and coefficients in natural units, where one unit is one
//! full-strength coupler. The wire and every machine record use integer milli. A
//! decimal converts only when it is an exact number of milli: nothing is rounded.

use serde_json::{Number, Value};

use super::super::contracts::EvalError;

const MILLI_PER_UNIT: i128 = 1000;
const MILLI_DECIMALS: i32 = 3;
/// A decimal of at most 15 significant digits survives a float round trip, so a
/// decimal's milli magnitude must stay below 10^15. Integers have no such limit.
const MAX_DECIMAL_MILLI: i128 = 1_000_000_000_000_000;

fn eval_error(code: &'static str, reason: impl Into<String>) -> EvalError {
    EvalError {
        code,
        reason: reason.into(),
    }
}

/// Convert a natural-unit JSON number to integer milli.
///
/// # Errors
/// `invalid_number` for a value that is not a JSON number, `unsupported_precision`
/// for more than three decimal places, and `overflow` for a result outside i64 or a
/// decimal beyond the range a JSON float carries exactly.
pub fn natural_to_milli(value: &Value) -> Result<i64, EvalError> {
    let Value::Number(number) = value else {
        return Err(eval_error(
            "invalid_number",
            format!("{value} is not a number"),
        ));
    };
    let milli = if let Some(units) = number.as_i64() {
        i128::from(units) * MILLI_PER_UNIT
    } else if number.is_u64() {
        return Err(eval_error("overflow", format!("{number} does not fit i64")));
    } else {
        decimal_to_milli(number)?
    };
    i64::try_from(milli).map_err(|_| eval_error("overflow", format!("{number} does not fit i64")))
}

/// `serde_json` prints the shortest decimal that reads back as the same float, so the
/// digits seen here are the digits of the file for every value this accepts.
fn decimal_to_milli(number: &Number) -> Result<i128, EvalError> {
    let text = number.to_string();
    let malformed = || eval_error("invalid_number", format!("{text} is not a decimal"));
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().map_err(|_| malformed())?),
        None => (text.as_str(), 0),
    };
    let (negative, unsigned) = match mantissa.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, mantissa),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let digits: i128 = format!("{whole}{fraction}")
        .parse()
        .map_err(|_| malformed())?;
    let fraction_len = i32::try_from(fraction.len()).map_err(|_| malformed())?;
    // value = digits * 10^(exponent - fraction_len), so milli = digits * 10^shift.
    let shift = exponent + MILLI_DECIMALS - fraction_len;
    let power = |exponent: i32| {
        u32::try_from(exponent)
            .ok()
            .and_then(|exponent| 10_i128.checked_pow(exponent))
            .ok_or_else(|| eval_error("overflow", format!("{text} is out of range")))
    };
    let magnitude = if shift >= 0 {
        digits
            .checked_mul(power(shift)?)
            .ok_or_else(|| eval_error("overflow", format!("{text} is out of range")))?
    } else {
        let divisor = power(-shift)?;
        if digits % divisor != 0 {
            return Err(eval_error(
                "unsupported_precision",
                format!("{text} has more than {MILLI_DECIMALS} decimal places"),
            ));
        }
        digits / divisor
    };
    if magnitude >= MAX_DECIMAL_MILLI {
        return Err(eval_error(
            "overflow",
            format!("{text} is beyond the exact decimal range; write an integer"),
        ));
    }
    Ok(if negative { -magnitude } else { magnitude })
}

/// Format integer milli as an exact natural-unit decimal, such as `-1.25` or `3`.
#[must_use]
pub fn milli_to_natural(milli: i64) -> String {
    let sign = if milli < 0 { "-" } else { "" };
    let magnitude = milli.unsigned_abs();
    let whole = magnitude / 1000;
    let fraction = magnitude % 1000;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let digits = format!("{fraction:03}");
        format!("{sign}{whole}.{}", digits.trim_end_matches('0'))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{milli_to_natural, natural_to_milli};

    #[test]
    fn decimals_convert_exactly_or_fail() {
        for (value, milli) in [
            (json!(1), 1000_i64),
            (json!(-0.001), -1),
            (json!(1.25), 1250),
            (json!(1e-3), 1),
            (json!(2.5e3), 2_500_000),
            (json!(-0.0), 0),
            (json!(999_999_999_999.999), 999_999_999_999_999),
            (json!(9_223_372_036_854_775_i64), 9_223_372_036_854_775_000),
        ] {
            assert_eq!(natural_to_milli(&value).ok(), Some(milli), "{value}");
        }
        for (value, code) in [
            (json!(0.0005), "unsupported_precision"),
            (json!(1e-4), "unsupported_precision"),
            (json!(9_223_372_036_854_776_i64), "overflow"),
            (json!(u64::MAX), "overflow"),
            (json!(9_007_199_254_740.993), "overflow"),
            (json!(1_000_000_000_000.001), "overflow"),
            (json!("1"), "invalid_number"),
        ] {
            assert_eq!(
                natural_to_milli(&value).err().map(|error| error.code),
                Some(code),
                "{value}"
            );
        }
    }

    #[test]
    fn milli_formats_as_natural() {
        for (milli, text) in [
            (0, "0"),
            (1000, "1"),
            (-1250, "-1.25"),
            (5, "0.005"),
            (-1, "-0.001"),
            (i64::MIN, "-9223372036854775.808"),
        ] {
            assert_eq!(milli_to_natural(milli), text);
        }
    }
}
