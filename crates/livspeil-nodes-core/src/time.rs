// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Time-of-day nodes.
//!
//! These nodes never read the system clock: the host feeds wall-clock values
//! (for example seconds since local midnight) through ordinary inputs, so
//! cooking stays a pure function of the inputs.

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

use crate::float;

/// Seconds in one day.
pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// Reject NaN and infinities on `port`.
pub(crate) fn finite(value: f64, port: &'static str) -> Result<f64, NodeError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(NodeError::InvalidInput {
            port,
            reason: format!("expected a finite number, got {value}"),
        })
    }
}

/// `x` wrapped into `[0, period)`.
///
/// `f64::rem_euclid` may return exactly `period` for tiny negative inputs
/// (rounding), so that case is folded back to 0.
pub(crate) fn wrap(x: f64, period: f64) -> f64 {
    let r = x.rem_euclid(period);
    if r >= period { 0.0 } else { r }
}

/// `core.time.timeofday@1`: turns seconds since local midnight into a
/// wrapping day phase and an hour of day.
///
/// Inputs: `seconds` (Float, seconds since local midnight, default 43200),
/// `offset` (Float, hours added before wrapping, default 0).
/// Outputs: `phase` (Float in `[0, 1)`, 0 = midnight, 0.5 = noon) and `hour`
/// (Float in `[0, 24)`). Any finite input wraps, including negative values
/// and values beyond one day; NaN or infinite inputs are rejected.
#[derive(Debug)]
pub struct TimeOfDay {
    spec: NodeSpec,
}

impl TimeOfDay {
    #[must_use]
    pub fn new() -> Self {
        TimeOfDay {
            spec: NodeSpec::new(
                "core.time.timeofday",
                1,
                "Day phase (0 = midnight, 0.5 = noon) and hour from seconds since local midnight",
            )
            .input("seconds", PortType::Float, Some(Value::Float(43_200.0)))
            .input("offset", PortType::Float, Some(Value::Float(0.0)))
            .output("phase", PortType::Float)
            .output("hour", PortType::Float),
        }
    }
}

impl Default for TimeOfDay {
    fn default() -> Self {
        Self::new()
    }
}

/// Phase in `[0, 1)` and hour in `[0, 24)` for `seconds` plus `offset_hours`.
#[must_use]
pub fn time_of_day(seconds: f64, offset_hours: f64) -> (f64, f64) {
    let s = wrap(seconds + offset_hours * 3_600.0, SECONDS_PER_DAY);
    let phase = wrap(s / SECONDS_PER_DAY, 1.0);
    let hour = wrap(s / 3_600.0, 24.0);
    (phase, hour)
}

impl NodeImpl for TimeOfDay {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let seconds = finite(float(inputs, 0, "seconds")?, "seconds")?;
        let offset = finite(float(inputs, 1, "offset")?, "offset")?;
        let (phase, hour) = time_of_day(seconds, offset);
        Ok(vec![Value::Float(phase), Value::Float(hour)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cook(seconds: f64, offset: f64) -> (f64, f64) {
        let out = TimeOfDay::new()
            .cook(
                &CookContext::default(),
                &[Value::Float(seconds), Value::Float(offset)],
            )
            .unwrap();
        (out[0].as_float().unwrap(), out[1].as_float().unwrap())
    }

    #[test]
    fn noon_and_midnight() {
        assert_eq!(cook(43_200.0, 0.0), (0.5, 12.0));
        assert_eq!(cook(0.0, 0.0), (0.0, 0.0));
        assert_eq!(cook(86_400.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn quarter_days() {
        assert_eq!(cook(21_600.0, 0.0), (0.25, 6.0));
        assert_eq!(cook(64_800.0, 0.0), (0.75, 18.0));
    }

    #[test]
    fn negative_and_large_inputs_wrap() {
        assert_eq!(cook(-3_600.0, 0.0), (23.0 / 24.0, 23.0));
        assert_eq!(cook(86_400.0 * 3.0 + 7_200.0, 0.0), (1.0 / 12.0, 2.0));
        assert_eq!(cook(-86_400.0 * 10.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn offset_is_in_hours() {
        assert_eq!(cook(0.0, 2.0), (1.0 / 12.0, 2.0));
        assert_eq!(cook(3_600.0, -2.0), (23.0 / 24.0, 23.0));
        assert_eq!(cook(43_200.0, 36.0), (0.0, 0.0));
    }

    #[test]
    fn tiny_negative_never_reaches_one() {
        let (phase, hour) = cook(-1e-12, 0.0);
        assert!((0.0..1.0).contains(&phase), "{phase}");
        assert!((0.0..24.0).contains(&hour), "{hour}");
        let (phase, hour) = cook(-f64::MIN_POSITIVE, 0.0);
        assert!((0.0..1.0).contains(&phase), "{phase}");
        assert!((0.0..24.0).contains(&hour), "{hour}");
    }

    #[test]
    fn non_finite_inputs_are_rejected() {
        let node = TimeOfDay::new();
        let err = node
            .cook(
                &CookContext::default(),
                &[Value::Float(f64::NAN), Value::Float(0.0)],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NodeError::InvalidInput {
                port: "seconds",
                ..
            }
        ));
        let err = node
            .cook(
                &CookContext::default(),
                &[Value::Float(0.0), Value::Float(f64::INFINITY)],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NodeError::InvalidInput { port: "offset", .. }
        ));
    }

    #[test]
    fn is_not_time_dependent() {
        assert!(!TimeOfDay::new().spec().time_dependent);
    }
}
