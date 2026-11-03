// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact conversion of family coefficient scales to the milli transport units.

use super::super::contracts::{EvalError, IsingEncoding};

/// Integer ticks per energy unit, where one unit is one full-strength coupler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegerScale {
    pub ticks_per_unit: u64,
}

impl IntegerScale {
    /// The archive and transport scale: 1000 ticks per unit.
    pub const MILLI: Self = Self {
        ticks_per_unit: 1000,
    };
}

fn eval_error(code: &'static str, reason: impl Into<String>) -> EvalError {
    EvalError {
        code,
        reason: reason.into(),
    }
}

fn to_milli(value: i64, ticks: u64) -> Result<i128, EvalError> {
    let scaled = i128::from(value) * i128::from(IntegerScale::MILLI.ticks_per_unit);
    let ticks = i128::from(ticks);
    if scaled % ticks != 0 {
        return Err(eval_error(
            "unsupported_precision",
            format!("{value} at {ticks} ticks per unit has no exact milli value"),
        ));
    }
    Ok(scaled / ticks)
}

fn narrow_i32(value: i128) -> Result<i32, EvalError> {
    i32::try_from(value)
        .map_err(|_| eval_error("overflow", "a converted coefficient does not fit i32"))
}

/// Convert `encoding`, whose coefficients and offset are in `scale` ticks, to milli.
///
/// # Errors
/// `invalid_scale` for zero ticks, `unsupported_precision` when a value has no exact
/// milli equivalent, and `overflow` when a coefficient leaves i32 or the offset or the
/// bound `abs(offset) + sum(abs(h)) + sum(abs(j))` leaves i64.
pub fn to_wire_encoding(
    encoding: &IsingEncoding,
    scale: IntegerScale,
) -> Result<IsingEncoding, EvalError> {
    if scale.ticks_per_unit == 0 {
        return Err(eval_error(
            "invalid_scale",
            "ticks_per_unit must be positive",
        ));
    }
    let ticks = scale.ticks_per_unit;
    let convert = |values: &[i32]| -> Result<Vec<i32>, EvalError> {
        values
            .iter()
            .map(|&value| narrow_i32(to_milli(i64::from(value), ticks)?))
            .collect()
    };
    let h = convert(&encoding.h)?;
    let j = convert(&encoding.j)?;
    let offset = to_milli(encoding.offset, ticks)?;
    let bound = h
        .iter()
        .chain(&j)
        .fold(offset.abs(), |sum, &value| sum + i128::from(value).abs());
    if i64::try_from(bound).is_err() {
        return Err(eval_error(
            "overflow",
            "the energy bound of the converted encoding does not fit i64",
        ));
    }
    Ok(IsingEncoding {
        h,
        j,
        offset: i64::try_from(offset).map_err(|_| eval_error("overflow", "offset"))?,
        units: "milli".to_owned(),
        ..encoding.clone()
    })
}
