// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sky nodes: where the sun is.
//!
//! # Coordinate convention
//!
//! Directions are in the scene's world frame: **+y is up, +x is east, +z is
//! south** (so −z is north). This is a right-handed frame (x × y = z gives
//! east × up = south). Azimuth is measured in degrees **clockwise from
//! north** when seen from above: 0 = north, 90 = east, 180 = south,
//! 270 = west.
//!
//! # Model and accuracy
//!
//! The solar declination uses **Cooper's formula** (1969):
//!
//! ```text
//! δ = 23.45° · sin(360° · (284 + N) / 365)
//! ```
//!
//! where `N` is the day of the year (1 = January 1). It is a single sine fit
//! that ignores the orbit's eccentricity and the leap-year cycle; its error
//! against the true declination is up to about 1° around the equinoxes and a
//! fraction of a degree near the solstices. The hour angle is
//! `H = 15° · (solar_hour − 12)`, with `solar_hour` given in local apparent
//! solar time, so the equation of time and the longitude correction are the
//! host's job. Atmospheric refraction is ignored. That is plenty for lighting
//! a desktop scene; it is not an ephemeris.

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

use crate::float;
use crate::time::{finite, wrap};

/// Horizontal length below which the azimuth is undefined (sun at the zenith
/// or nadir); the node then reports azimuth 0.
pub const AZIMUTH_EPSILON: f64 = 1e-12;

/// Azimuths within this many degrees below 360 are reported as 0, so that
/// rounding noise in a due-north sun (for example `sin(π)` not being exactly
/// zero at solar midnight) cannot flip the result between 0 and 359.999….
pub const AZIMUTH_SEAM_EPSILON: f64 = 1e-9;

/// The sun's position for one day, time and latitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunPosition {
    /// Unit vector toward the sun, `[east, up, south]`.
    pub direction: [f64; 3],
    /// Degrees above the horizon, in `[-90, 90]`.
    pub elevation: f64,
    /// Degrees clockwise from north, in `[0, 360)`.
    pub azimuth: f64,
}

/// Solar declination in degrees (Cooper's formula) for `day_of_year`.
#[must_use]
pub fn declination_degrees(day_of_year: f64) -> f64 {
    23.45 * (360.0 * (284.0 + day_of_year) / 365.0).to_radians().sin()
}

/// Sun position; `latitude` in degrees must already be in `[-90, 90]`.
#[must_use]
pub fn sun_position(day_of_year: f64, solar_hour: f64, latitude: f64) -> SunPosition {
    let decl = declination_degrees(day_of_year).to_radians();
    let hour_angle = (15.0 * (solar_hour - 12.0)).to_radians();
    let lat = latitude.to_radians();

    // Local east/north/up components. A positive hour angle (afternoon) puts
    // the sun in the west, hence the minus sign on east.
    let east = -decl.cos() * hour_angle.sin();
    let north = decl.sin() * lat.cos() - decl.cos() * hour_angle.cos() * lat.sin();
    let up = decl.sin() * lat.sin() + decl.cos() * hour_angle.cos() * lat.cos();

    // Mathematically unit length already; renormalize away rounding.
    let len = (east * east + north * north + up * up).sqrt();
    let (east, north, up) = (east / len, north / len, up / len);

    let elevation = up.clamp(-1.0, 1.0).asin().to_degrees();
    let azimuth = if east.hypot(north) < AZIMUTH_EPSILON {
        0.0
    } else {
        let az = wrap(east.atan2(north).to_degrees(), 360.0);
        if az > 360.0 - AZIMUTH_SEAM_EPSILON {
            0.0
        } else {
            az
        }
    };

    SunPosition {
        direction: [east, up, -north],
        elevation,
        azimuth,
    }
}

/// `core.sky.sun_direction@1`: the direction, elevation and azimuth of the
/// sun from the day of the year, local solar time and latitude.
///
/// Inputs: `day_of_year` (Float, default 172 = June solstice; must lie in
/// `[1, 366]`, fractional days allowed; anything else, including `NaN` and
/// infinities, is [`NodeError::InvalidInput`]),
/// `solar_hour` (Float, local apparent solar time 0..24, default 12),
/// `latitude` (Float degrees, north positive, default 41.7; outside
/// `[-90, 90]` is [`NodeError::InvalidInput`]).
/// Outputs: `direction` (Vec3, unit, see the module docs for the axes),
/// `elevation` (Float degrees), `azimuth` (Float degrees clockwise from
/// north).
#[derive(Debug)]
pub struct SunDirection {
    spec: NodeSpec,
}

impl SunDirection {
    #[must_use]
    pub fn new() -> Self {
        SunDirection {
            spec: NodeSpec::new(
                "core.sky.sun_direction",
                1,
                "Sun direction (y up, x east, z south), elevation and azimuth from day, solar time and latitude",
            )
            .input("day_of_year", PortType::Float, Some(Value::Float(172.0)))
            .input("solar_hour", PortType::Float, Some(Value::Float(12.0)))
            .input("latitude", PortType::Float, Some(Value::Float(41.7)))
            .output("direction", PortType::Vec3)
            .output("elevation", PortType::Float)
            .output("azimuth", PortType::Float),
        }
    }
}

impl Default for SunDirection {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for SunDirection {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let day = float(inputs, 0, "day_of_year")?;
        if !(1.0..=366.0).contains(&day) {
            return Err(NodeError::InvalidInput {
                port: "day_of_year",
                reason: format!("day_of_year must be within [1, 366], got {day}"),
            });
        }
        let hour = finite(float(inputs, 1, "solar_hour")?, "solar_hour")?;
        let latitude = float(inputs, 2, "latitude")?;
        if !(-90.0..=90.0).contains(&latitude) {
            return Err(NodeError::InvalidInput {
                port: "latitude",
                reason: format!("latitude must be within [-90, 90] degrees, got {latitude}"),
            });
        }
        let sun = sun_position(day, hour, latitude);
        Ok(vec![
            Value::Vec3(sun.direction),
            Value::Float(sun.elevation),
            Value::Float(sun.azimuth),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JUNE_SOLSTICE: f64 = 172.0;
    const DECEMBER_SOLSTICE: f64 = 355.0;
    const MARCH_EQUINOX: f64 = 80.0;

    fn cook(day: f64, hour: f64, lat: f64) -> SunPosition {
        let out = SunDirection::new()
            .cook(
                &CookContext::default(),
                &[Value::Float(day), Value::Float(hour), Value::Float(lat)],
            )
            .unwrap();
        let Value::Vec3(direction) = out[0] else {
            panic!("direction is not a Vec3: {:?}", out[0]);
        };
        SunPosition {
            direction,
            elevation: out[1].as_float().unwrap(),
            azimuth: out[2].as_float().unwrap(),
        }
    }

    fn close(actual: f64, expected: f64, tol: f64) {
        assert!(
            (actual - expected).abs() <= tol,
            "expected {expected} ± {tol}, got {actual}"
        );
    }

    #[test]
    fn declination_peaks_at_the_solstices() {
        close(declination_degrees(JUNE_SOLSTICE), 23.45, 0.01);
        close(declination_degrees(DECEMBER_SOLSTICE), -23.45, 0.01);
        close(declination_degrees(MARCH_EQUINOX), 0.0, 1.0);
    }

    #[test]
    fn equator_equinox_noon_is_overhead() {
        let sun = cook(MARCH_EQUINOX, 12.0, 0.0);
        close(sun.elevation, 90.0, 1.0);
        assert!(sun.direction[1] > 0.9998);
    }

    #[test]
    fn mid_latitude_solstice_noons() {
        close(cook(JUNE_SOLSTICE, 12.0, 41.7).elevation, 71.7, 0.1);
        close(cook(DECEMBER_SOLSTICE, 12.0, 41.7).elevation, 24.9, 0.1);
    }

    #[test]
    fn noon_sun_is_due_south_in_the_north() {
        for day in [1.0, MARCH_EQUINOX, JUNE_SOLSTICE, DECEMBER_SOLSTICE] {
            let sun = cook(day, 12.0, 41.7);
            close(sun.azimuth, 180.0, 1e-9);
            // South is +z.
            assert!(sun.direction[2] > 0.0);
            close(sun.direction[0], 0.0, 1e-12);
        }
    }

    #[test]
    fn noon_sun_is_due_north_in_the_south() {
        let sun = cook(JUNE_SOLSTICE, 12.0, -33.9);
        close(sun.azimuth, 0.0, 1e-9);
        assert!(sun.direction[2] < 0.0);
    }

    #[test]
    fn morning_is_east_and_afternoon_is_west() {
        let morning = cook(JUNE_SOLSTICE, 9.0, 41.7);
        let afternoon = cook(JUNE_SOLSTICE, 15.0, 41.7);
        assert!(morning.direction[0] > 0.0);
        assert!(afternoon.direction[0] < 0.0);
        assert!((0.0..180.0).contains(&morning.azimuth));
        assert!((180.0..360.0).contains(&afternoon.azimuth));
        // Symmetric around solar noon.
        close(morning.elevation, afternoon.elevation, 1e-9);
        close(morning.azimuth, 360.0 - afternoon.azimuth, 1e-9);
    }

    #[test]
    fn midnight_is_below_the_horizon() {
        let sun = cook(DECEMBER_SOLSTICE, 0.0, 41.7);
        assert!(sun.elevation < 0.0);
        assert!(sun.direction[1] < 0.0);
    }

    #[test]
    fn direction_is_unit_and_matches_angles() {
        for day in [1.0, 45.5, MARCH_EQUINOX, JUNE_SOLSTICE, 266.0, 366.0] {
            for hour in [0.0, 3.25, 6.0, 9.5, 12.0, 14.0, 18.75, 23.9] {
                for lat in [-90.0, -60.0, -23.0, 0.0, 41.7, 66.6, 89.0, 90.0] {
                    let sun = cook(day, hour, lat);
                    let [x, y, z] = sun.direction;
                    close((x * x + y * y + z * z).sqrt(), 1.0, 1e-12);
                    assert!((-90.0..=90.0).contains(&sun.elevation));
                    assert!((0.0..360.0).contains(&sun.azimuth));
                    let el = sun.elevation.to_radians();
                    let az = sun.azimuth.to_radians();
                    close(y, el.sin(), 1e-9);
                    if el.cos() > 1e-6 {
                        close(x, el.cos() * az.sin(), 1e-9);
                        close(z, -el.cos() * az.cos(), 1e-9);
                    }
                }
            }
        }
    }

    #[test]
    fn latitude_out_of_range_is_rejected() {
        for lat in [90.5, -91.0, f64::NAN, f64::INFINITY] {
            let err = SunDirection::new()
                .cook(
                    &CookContext::default(),
                    &[Value::Float(172.0), Value::Float(12.0), Value::Float(lat)],
                )
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    NodeError::InvalidInput {
                        port: "latitude",
                        ..
                    }
                ),
                "{lat}"
            );
        }
    }

    #[test]
    fn day_of_year_out_of_range_is_rejected() {
        for day in [0.0, 0.999, 366.001, 367.0, -1.0, f64::NAN, f64::INFINITY] {
            let err = SunDirection::new()
                .cook(
                    &CookContext::default(),
                    &[Value::Float(day), Value::Float(12.0), Value::Float(41.7)],
                )
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    NodeError::InvalidInput {
                        port: "day_of_year",
                        ..
                    }
                ),
                "{day}"
            );
        }
        // Both ends are inclusive.
        for day in [1.0, 366.0] {
            let sun = cook(day, 12.0, 41.7);
            assert!((0.0..360.0).contains(&sun.azimuth));
        }
    }

    #[test]
    fn due_north_azimuth_is_zero_not_360() {
        // Solar midnight: the hour angle is ±180°, whose sine is rounding
        // noise of either sign.
        for hour in [0.0, 24.0, 48.0] {
            let sun = cook(JUNE_SOLSTICE, hour, 41.7);
            assert!(sun.azimuth < 1e-9, "{hour}: {}", sun.azimuth);
        }
    }

    #[test]
    fn poles_report_a_defined_azimuth() {
        // At the pole every horizontal direction is "south" (or "north");
        // the azimuth must still be a finite value in range.
        let sun = cook(JUNE_SOLSTICE, 12.0, 90.0);
        close(sun.elevation, 23.45, 0.01);
        assert!((0.0..360.0).contains(&sun.azimuth));
    }
}
