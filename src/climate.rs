//! Seasonal climate model.
//!
//! Temperature varies with **latitude** (north = top/`y=0`, south = bottom/
//! `y=height`) and with the **season**, which cycles as the simulation runs.
//!
//! - South is hotter than north.
//! - Summer is hotter everywhere; winter is colder everywhere.
//!
//! So winter is harsh in the north but *dampened* in the warm south, while
//! summer makes the south punishingly hot — exactly the gradient creatures must
//! migrate across or genetically adapt to.

use std::f64::consts::TAU;

/// Climate shaping constants (pulled from [`crate::config::Config`]).
#[derive(Clone, Copy, Debug)]
pub struct ClimateParams {
    /// Baseline temperature (the global average).
    pub base: f64,
    /// How much warmer the far south is than the far north (peak-to-peak).
    pub south_amp: f64,
    /// How much hotter peak summer is than the yearly average.
    pub season_amp: f64,
    /// Cycles in one full year (controls how fast seasons turn).
    pub cycles_per_year: f64,
}

/// Seasonal phase in radians for a given cycle.
pub fn season_phase(cycle: u64, cycles_per_year: f64) -> f64 {
    if cycles_per_year <= 0.0 {
        0.0
    } else {
        (cycle as f64 / cycles_per_year) * TAU
    }
}

/// Season indicator in `[-1, 1]`: `+1` = peak summer, `-1` = peak winter.
pub fn seasonality(phase: f64) -> f64 {
    phase.sin()
}

/// Local temperature at latitude `y` for the current `phase`.
/// `y = 0` is the north (cold edge), `y = height` is the south (warm edge).
pub fn temperature(y: f64, height: f64, phase: f64, p: &ClimateParams) -> f64 {
    let lat = if height > 0.0 {
        (y / height).clamp(0.0, 1.0)
    } else {
        0.5
    };
    // lat 0 (north) -> -south_amp/2, lat 1 (south) -> +south_amp/2.
    p.base + p.south_amp * (lat - 0.5) + p.season_amp * seasonality(phase)
}

/// The latitude (`y`) food-bearing trees drift toward this season: north in
/// summer, south in winter.
pub fn tree_target_y(height: f64, phase: f64, migrate_amp: f64) -> f64 {
    // summer (sin=+1) -> lat 0.5 - amp (north); winter (sin=-1) -> 0.5 + amp.
    let lat = (0.5 - migrate_amp * seasonality(phase)).clamp(0.0, 1.0);
    lat * height
}

/// Human-readable season label (four seasons from the phase).
pub fn season_name(phase: f64) -> &'static str {
    let s = phase.sin();
    let c = phase.cos();
    if s > 0.5 {
        "Summer"
    } else if s < -0.5 {
        "Winter"
    } else if c > 0.0 {
        "Spring" // warming toward summer
    } else {
        "Autumn" // cooling toward winter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ClimateParams {
        ClimateParams {
            base: 0.5,
            south_amp: 0.6,
            season_amp: 0.6,
            cycles_per_year: 8.0,
        }
    }

    #[test]
    fn south_is_warmer_than_north() {
        let p = params();
        let phase = 0.0;
        let north = temperature(0.0, 40.0, phase, &p);
        let south = temperature(40.0, 40.0, phase, &p);
        assert!(south > north);
    }

    #[test]
    fn summer_is_warmer_than_winter() {
        let p = params();
        let summer = season_phase(2, p.cycles_per_year); // quarter year -> sin=1
        let winter = season_phase(6, p.cycles_per_year); // three-quarter -> sin=-1
        let y = 20.0;
        assert!(temperature(y, 40.0, summer, &p) > temperature(y, 40.0, winter, &p));
    }

    #[test]
    fn winter_is_dampened_in_the_south() {
        // The spec's core claim: winter cold is milder in the warm south.
        let p = params();
        let winter = season_phase(6, p.cycles_per_year);
        let north = temperature(0.0, 40.0, winter, &p);
        let south = temperature(40.0, 40.0, winter, &p);
        assert!(south > north);
    }

    #[test]
    fn trees_go_north_in_summer_south_in_winter() {
        let p = params();
        let summer = season_phase(2, p.cycles_per_year);
        let winter = season_phase(6, p.cycles_per_year);
        let north_target = tree_target_y(40.0, summer, 0.4);
        let south_target = tree_target_y(40.0, winter, 0.4);
        assert!(north_target < south_target);
    }
}
