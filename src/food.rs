//! Food sources (trees / bushes), the fruit they drop, and aquatic plants.
//!
//! ## Trees
//! Trees have their own evolving genome. New heritable traits:
//! - **`seed_dispersal`**: multiplies how far fruit lands from the parent.
//!   Wide dispersal colonises new ground; tight dispersal keeps density high.
//! - **`drought_resist`**: fraction of fruit production that survives summer heat.
//!   High-resist trees are barely affected by droughts; low-resist ones go
//!   almost silent in hot, dry summer peaks.
//! - **`canopy_competition`**: how aggressively this tree shades its neighbours.
//!   High-competition trees suppress nearby rivals; low-competition trees coexist
//!   at higher density but lose ground to aggressive colonisers.
//!
//! ## Aquatic plants
//! Shallow and deep water tiles can host `AquaticPlant` patches — analogous to
//! kelp / algae. They grow continuously and are available year-round (much less
//! seasonally variable than land fruit), rewarding creatures that invest in
//! `swim_capability`.

use crate::climate;
use crate::geometry::Vec2;
use crate::rng::Rng;

// ─────────────────────────────────────────────────────────────────────────────
// TreeGenome
// ─────────────────────────────────────────────────────────────────────────────

/// Heritable traits of a food-source tree.
#[derive(Clone, Copy, Debug)]
pub struct TreeGenome {
    /// Toxicity of the fruit, `0.0..1.0` (damages non-resistant eaters).
    pub poison: f64,
    /// Base energy stored in each fruit (modulated by season at drop time).
    pub fruit_energy: f64,
    /// Fruiting-rate multiplier (fruit dropped per tick, at ideal climate).
    pub fertility: f64,
    /// Climate this tree is adapted to (preferred temperature, same scale as
    /// the world temperature output).
    pub temp_optimum: f64,

    // ── New heritable traits ────────────────────────────────────────────────
    /// Fruit-scatter radius multiplier: `1.0` = default scatter, `3.0` = wide
    /// dispersal into new territory, `0.3` = tight local cluster.
    pub seed_dispersal: f64,
    /// Resilience to summer drought `[0, 1]`: how much of base fruiting rate
    /// is preserved when the season is hot and dry. A value of `1.0` means the
    /// tree is completely unaffected by heat spikes; `0.0` means it goes fully
    /// dormant in peak summer heat.
    pub drought_resist: f64,
    /// Competitive shading strength `[0, 1]`: how much this tree suppresses the
    /// fertility of nearby trees. High-competition trees dominate clearings;
    /// low-competition trees survive better in dense forests.
    pub canopy_competition: f64,
    /// Climate adaptability: preferred temperature tolerance range (comfort band width).
    /// Baseline `0.4..1.0`. Higher tolerance has a mild fertility discount tradeoff.
    pub temp_tolerance: f64,
}

impl TreeGenome {
    pub fn random(rng: &mut Rng) -> Self {
        TreeGenome {
            poison:            rng.range_f64(0.0, 0.2),
            fruit_energy:      rng.range_f64(30.0, 55.0),
            fertility:         rng.range_f64(0.8, 1.4),
            temp_optimum:      rng.range_f64(0.2, 0.8),
            seed_dispersal:    rng.range_f64(0.5, 2.0),
            drought_resist:    rng.range_f64(0.2, 0.8),
            canopy_competition:rng.range_f64(0.1, 0.6),
            temp_tolerance:    rng.range_f64(0.4, 1.0),
        }
    }

    fn clamp(&mut self) {
        self.poison             = self.poison.clamp(0.0, 1.0);
        self.fruit_energy       = self.fruit_energy.clamp(10.0, 90.0);
        self.fertility          = self.fertility.clamp(0.1, 3.0);
        self.temp_optimum       = self.temp_optimum.clamp(-1.0, 2.0);
        self.seed_dispersal     = self.seed_dispersal.clamp(0.2, 4.0);
        self.drought_resist     = self.drought_resist.clamp(0.0, 1.0);
        self.canopy_competition = self.canopy_competition.clamp(0.0, 1.0);
        self.temp_tolerance     = self.temp_tolerance.clamp(0.1, 2.0);
    }

    /// Mutated copy used when fruit germinates into a new tree.
    pub fn mutated(&self, rng: &mut Rng) -> TreeGenome {
        let mut g = *self;
        g.poison             += rng.gaussian() * 0.04;
        g.fruit_energy       += rng.gaussian() * 3.0;
        g.fertility          += rng.gaussian() * 0.08;
        g.temp_optimum       += rng.gaussian() * 0.06;
        g.seed_dispersal     += rng.gaussian() * 0.15;
        g.drought_resist     += rng.gaussian() * 0.06;
        g.canopy_competition += rng.gaussian() * 0.06;
        g.temp_tolerance     += rng.gaussian() * 0.05;
        g.clamp();
        g
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FoodSource (tree)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FoodSource {
    pub pos: Vec2,
    pub genome: TreeGenome,
    /// Unique identifier; assigned by the World after construction.
    pub id: u64,
    /// How many of this tree's fruits were eaten this cycle.
    /// Reset at end-of-cycle; used to evolve toxicity under grazing pressure.
    pub feeds_this_cycle: u32,
    pub is_fruit_tree: bool,
}

impl FoodSource {
    pub fn new(pos: Vec2, genome: TreeGenome) -> Self {
        FoodSource { pos, genome, id: 0, feeds_this_cycle: 0, is_fruit_tree: true }
    }

    /// Climate fitness `[0, 1]`: trees far from their preferred temperature fruit
    /// poorly. The comfort band is based on evolved temp_tolerance.
    pub fn climate_fit(&self, temp: f64) -> f64 {
        let dev = (temp - self.genome.temp_optimum).abs();
        let tol = self.genome.temp_tolerance.max(0.1);
        (1.0 - dev / tol).clamp(0.0, 1.0)
    }

    /// Effective fruiting rate given current season.
    ///
    /// - `base_rate`: the per-tick fruit drop rate from config.
    /// - `seasonality`: the `sin(phase)` value `[-1, +1]`; +1 = peak summer,
    ///   -1 = peak winter.
    /// - `fert_boost`: extra multiplier from nearby fertilizer.
    /// - `canopy_suppression`: fertility lost to neighbouring trees' shading.
    ///
    /// **Winter** production drops toward `winter_floor` (a fraction of base).
    /// **Summer heat** further reduces production for low-drought-resist trees:
    /// very hot summers cause partial dormancy.
    /// **Autumn** is peak productivity: seasonality ≈ 0 while cosine is < 0.
    pub fn fruiting_rate(
        &self,
        base_rate: f64,
        temp: f64,
        seasonality: f64,     // sin(phase): +1=summer, -1=winter
        fert_boost: f64,
        canopy_suppression: f64, // 0..1 (0 = no suppression)
        winter_floor: f64,    // min fraction of rate in deep winter
        summer_heat_scale: f64, // how much heat reduces drought-sensitive trees
    ) -> f64 {
        let climate = self.climate_fit(temp);

        // Seasonal factor: winter drops to `winter_floor` fraction of base,
        // summer raises by up to +50% for well-adapted trees.
        let seasonal_factor = if seasonality < 0.0 {
            // Winter: lerp between 1.0 and winter_floor as |seasonality| → 1.
            1.0 + seasonality * (1.0 - winter_floor) // when s=-1 → winter_floor
        } else {
            // Summer: base + extra; heat-sensitive trees get a drought penalty.
            let heat_penalty = seasonality * summer_heat_scale * (1.0 - self.genome.drought_resist);
            1.0 + seasonality * 0.5 - heat_penalty
        }
        .max(winter_floor);

        let canopy = (1.0 - canopy_suppression).max(0.1);
        // Upkeep discount for high climate tolerance
        let eff_fertility = self.genome.fertility * (1.1 - self.genome.temp_tolerance * 0.2);
        base_rate * eff_fertility * climate * fert_boost * seasonal_factor * canopy
    }

    /// Drift toward this season's favourable latitude; same as before.
    pub fn migrate(
        &mut self,
        distance: f64,
        w: f64,
        h: f64,
        phase: f64,
        migrate_amp: f64,
        rng: &mut Rng,
    ) {
        let target_y = climate::tree_target_y(h, phase, migrate_amp);
        let dy = (target_y - self.pos.y).clamp(-distance, distance);
        let remaining = (distance * distance - dy * dy).max(0.0).sqrt();
        let dx = rng.range_f64(-remaining, remaining);
        let next = Vec2::new(
            self.pos.x + dx,
            self.pos.y + dy * rng.range_f64(0.6, 1.0),
        );
        self.pos = next.clamp_to(w, h);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fruit
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Fruit {
    pub pos: Vec2,
    /// Energy, scaled at drop time by season (autumn-ripe, spring-lean).
    pub energy: f64,
    /// Toxicity inherited from the parent tree.
    pub poison: f64,
    /// Parent genome carried so the fruit can germinate true-to-type.
    pub parent: TreeGenome,
    /// Cycles on the ground (eligible to germinate once old enough).
    pub age_cycles: u32,
    /// ID of the tree that dropped this fruit.
    pub source_id: u64,
}

impl Fruit {
    /// `season_energy_scale`: typically in `[0.5, 1.3]` — autumn fruit is riper,
    /// spring fruit is nutrient-depleted.
    pub fn new(pos: Vec2, parent: TreeGenome, source_id: u64, season_energy_scale: f64) -> Self {
        Fruit {
            pos,
            energy: (parent.fruit_energy * season_energy_scale).max(5.0),
            poison: parent.poison,
            parent,
            age_cycles: 0,
            source_id,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AquaticPlant
// ─────────────────────────────────────────────────────────────────────────────

/// A patch of aquatic vegetation (kelp, algae, seagrass) that grows in shallow
/// and deep water tiles and provides food for creatures with swimming ability.
///
/// Unlike land fruit, aquatic plants are not dropped by a tree — they regrow
/// on their own up to `max_energy`, making water tiles permanently productive
/// for adapted creatures. They are not significantly affected by seasons (water
/// temperature is buffered), though deep-cold tiles grow a bit more slowly.
#[derive(Clone, Debug)]
pub struct AquaticPlant {
    pub pos: Vec2,
    /// Current stored energy (eaten-down by herbivorous/omnivorous swimmers).
    pub energy: f64,
    /// Maximum energy this patch can hold.
    pub max_energy: f64,
    /// Energy regained per tick (grows back after being grazed).
    pub regrow_rate: f64,
    /// Whether this is in deep water (grows slower, harder to reach without
    /// very high swim_capability).
    pub deep: bool,
}

impl AquaticPlant {
    pub fn new(pos: Vec2, max_energy: f64, regrow_rate: f64, deep: bool) -> Self {
        AquaticPlant {
            pos,
            // Start half-full so there's food from tick 1.
            energy: max_energy * 0.5,
            max_energy,
            regrow_rate,
            deep,
        }
    }

    /// Regrow energy by one tick.
    pub fn tick_regrow(&mut self) {
        self.energy = (self.energy + self.regrow_rate).min(self.max_energy);
    }

    /// Bite up to `bite` energy; returns how much was actually consumed.
    pub fn bite(&mut self, bite: f64) -> f64 {
        let taken = bite.min(self.energy);
        self.energy -= taken;
        taken
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> FoodSource {
        let mut rng = Rng::new(1);
        FoodSource::new(Vec2::new(40.0, 20.0), TreeGenome::random(&mut rng))
    }

    #[test]
    fn climate_fit_peaks_at_optimum() {
        let mut t = tree();
        t.genome.temp_optimum = 0.5;
        assert!((t.climate_fit(0.5) - 1.0).abs() < 1e-9);
        assert!(t.climate_fit(0.5) > t.climate_fit(1.5));
    }

    #[test]
    fn migrate_stays_in_bounds() {
        let mut rng = Rng::new(2);
        let mut t = tree();
        for cycle in 0..200 {
            let phase = climate::season_phase(cycle, 8.0);
            t.migrate(12.0, 80.0, 40.0, phase, 0.4, &mut rng);
            assert!(t.pos.x >= 0.0 && t.pos.x < 80.0);
            assert!(t.pos.y >= 0.0 && t.pos.y < 40.0);
        }
    }

    #[test]
    fn tree_mutation_stays_in_range() {
        let mut rng = Rng::new(3);
        let mut g = TreeGenome::random(&mut rng);
        for _ in 0..500 {
            g = g.mutated(&mut rng);
            assert!(g.poison >= 0.0 && g.poison <= 1.0);
            assert!(g.fruit_energy >= 10.0 && g.fruit_energy <= 90.0);
            assert!(g.seed_dispersal >= 0.2 && g.seed_dispersal <= 4.0);
            assert!(g.drought_resist >= 0.0 && g.drought_resist <= 1.0);
            assert!(g.canopy_competition >= 0.0 && g.canopy_competition <= 1.0);
            assert!(g.temp_tolerance >= 0.1 && g.temp_tolerance <= 2.0);
        }
    }

    #[test]
    fn winter_reduces_fruiting_rate() {
        let t = tree();
        let summer = t.fruiting_rate(1.0, 0.5, 1.0, 1.0, 0.0, 0.1, 0.3);
        let winter = t.fruiting_rate(1.0, 0.5, -1.0, 1.0, 0.0, 0.1, 0.3);
        assert!(summer > winter, "summer should produce more than winter");
    }

    #[test]
    fn high_drought_resist_survives_summer() {
        let mut rng = Rng::new(1);
        let mut g = TreeGenome::random(&mut rng);
        g.drought_resist = 1.0;
        let t_resist = FoodSource::new(Vec2::zero(), g);
        g.drought_resist = 0.0;
        let t_fragile = FoodSource::new(Vec2::zero(), g);
        let resist_rate = t_resist.fruiting_rate(1.0, 0.5, 1.0, 1.0, 0.0, 0.1, 1.0);
        let fragile_rate = t_fragile.fruiting_rate(1.0, 0.5, 1.0, 1.0, 0.0, 0.1, 1.0);
        assert!(resist_rate > fragile_rate, "drought-resistant trees should fruit more in summer heat");
    }

    #[test]
    fn aquatic_plant_regrows() {
        let mut p = AquaticPlant::new(Vec2::zero(), 60.0, 2.0, false);
        p.energy = 0.0;
        for _ in 0..30 {
            p.tick_regrow();
        }
        assert!(p.energy >= 60.0 - 1e-6);
    }

    #[test]
    fn aquatic_plant_caps_at_max() {
        let mut p = AquaticPlant::new(Vec2::zero(), 50.0, 100.0, false);
        p.tick_regrow();
        assert!((p.energy - 50.0).abs() < 1e-9);
    }
}
