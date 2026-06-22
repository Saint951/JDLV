//! Tunable parameters for a simulation run.
//!
//! Defaults are hand-tuned so a run neither dies out instantly nor explodes,
//! giving evolution room to act. Everything is overridable from the CLI.

#[derive(Clone, Debug)]
pub struct Config {
    pub seed: u64,

    // World geometry.
    pub width: f64,
    pub height: f64,

    // Time structure: a "cycle" is `ticks_per_cycle` ticks long.
    pub ticks_per_cycle: u64,
    pub max_cycles: u64,

    // Starting population.
    pub initial_creatures: usize,
    pub initial_food_sources: usize,
    /// Fruit pre-scattered around each source at world creation.
    pub initial_fruit_per_source: usize,

    // Death roll at the end of every cycle:
    //   p(death) = base + per_cycle_survived * cycles_survived, capped.
    pub death_base: f64,
    pub death_per_cycle: f64,
    pub death_cap: f64,

    // Food-source behaviour.
    pub source_relocate_distance: f64,
    pub fruit_drop_per_tick: f64,
    pub fruit_energy: f64,
    pub fruit_scatter: f64,        // base scatter; multiplied by tree seed_dispersal gene
    pub max_fruits: usize,

    // Energy economy.
    pub start_energy: f64,
    pub offspring_energy: f64,
    pub asexual_repro_cost: f64,
    pub sexual_repro_cost_per_parent: f64,
    pub starve_threshold: f64,
    pub max_population: usize,

    // --- Climate & seasons (now harsher) ---
    pub cycles_per_year: f64,
    pub climate_base: f64,
    pub south_warm_amp: f64,
    /// Seasonal temperature swing (peak summer vs peak winter). Higher = harsher.
    pub season_amp: f64,
    pub climate_penalty: f64,
    pub tree_migrate_amp: f64,
    /// Fraction of fruiting rate preserved in deep winter (0.0 = total dormancy).
    pub winter_fruit_floor: f64,
    /// Strength of summer-heat drought effect on low-drought-resist trees (0..1).
    pub summer_heat_scale: f64,
    /// Energy multiplier on fruit in autumn (peak ripeness) vs spring (lean).
    /// Applied at the time of fruit drop: autumn ≈ 1.3, spring ≈ 0.65.
    pub autumn_energy_bonus: f64,
    pub spring_energy_penalty: f64,

    // --- Carcasses & decomposition ---
    pub carcass_body_factor: f64,
    pub carcass_decay: f64,
    pub max_carcasses: usize,

    // --- Fertilizer & tree growth ---
    pub poop_chance: f64,
    pub poop_amount: f64,
    pub fertilizer_decay: f64,
    pub max_fertilizer: usize,
    pub fruit_germinate_age: u32,
    pub germinate_base_chance: f64,
    pub germinate_fert_boost: f64,
    pub max_trees: usize,
    pub tree_death_base: f64,
    pub tree_death_climate: f64,
    pub min_trees: usize,
    /// Radius within which two trees suppress each other's fertility via canopy
    /// competition (the canopy_competition gene scales the effect).
    pub canopy_competition_radius: f64,

    // --- Terrain ---
    pub mountain_cold_offset: f64,

    // --- Toxins ---
    pub poison_damage: f64,
    pub tree_heavy_graze_threshold: u32,

    // --- Social vectors ---
    pub social_influence_radius: f64,

    // --- Aquatic ecosystem ---
    /// How many aquatic-plant patches to seed at world start.
    pub initial_aquatic_plants: usize,
    /// Maximum aquatic-plant patches allowed world-wide.
    pub max_aquatic_plants: usize,
    /// Per-tick new aquatic plant spawn chance (adds patches to open water).
    pub aquatic_plant_spawn_rate: f64,
    /// Max energy per aquatic-plant patch.
    pub aquatic_plant_max_energy: f64,
    /// Energy regrown per patch per tick (shallow water; deep is ×0.6).
    pub aquatic_plant_regrow_rate: f64,
    /// Minimum swim_capability to eat shallow-water aquatic plants.
    pub aquatic_eat_shallow_min_swim: f64,
    /// Minimum swim_capability to eat deep-water aquatic plants.
    pub aquatic_eat_deep_min_swim: f64,
    /// Speed multiplier bonus for high-swim creatures in water tiles.
    pub water_speed_bonus: f64,
    /// How much swim_capability suppresses prey's effective sense radius in water
    /// (stealthy aquatic ambush: 0 = no suppression, 1 = halved sense).
    pub water_stealth_factor: f64,

    // --- Pack hunting ---
    /// Extra combat-power fraction added per additional pack member (0.6 = 60%).
    pub pack_hunt_bonus: f64,
    /// Maximum pack size that counts toward bonus.
    pub pack_max_size: usize,

    // --- Grass & Grazing ---
    pub grass_regrow_rate: f64,
    pub grass_max_energy: f64,
    pub grass_graze_max: f64,
    pub sociability_wander_bias: f64,

    // Rendering.
    pub render_cols: usize,
    pub render_rows: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            seed: 1,

            // 10x larger world area
            width:  1000.0,
            height: 500.0,

            ticks_per_cycle: 30,
            max_cycles: 80,

            initial_creatures: 800,
            initial_food_sources: 150,
            initial_fruit_per_source: 14,

            death_base: 0.20,
            death_per_cycle: 0.04,
            death_cap: 1.0,

            source_relocate_distance: 16.0,
            fruit_drop_per_tick: 0.8,
            fruit_energy: 35.0,
            fruit_scatter: 5.0,
            max_fruits: 10000,

            start_energy: 150.0,
            offspring_energy: 80.0,
            asexual_repro_cost: 80.0,
            sexual_repro_cost_per_parent: 20.0,
            starve_threshold: 0.0,
            max_population: 12000,

            // Harsher seasons
            cycles_per_year: 8.0,
            climate_base: 0.5,
            south_warm_amp: 0.9,
            season_amp: 1.1,
            climate_penalty: 1.4,
            tree_migrate_amp: 0.45,
            winter_fruit_floor: 0.08,
            summer_heat_scale: 0.7,
            autumn_energy_bonus: 1.35,
            spring_energy_penalty: 0.60,

            carcass_body_factor: 90.0,
            carcass_decay: 0.18,
            max_carcasses: 4000,

            poop_chance: 0.05,
            poop_amount: 1.0,
            fertilizer_decay: 0.30,
            max_fertilizer: 2400,
            fruit_germinate_age: 2,
            germinate_base_chance: 0.11,
            germinate_fert_boost: 0.06,
            max_trees: 600,
            tree_death_base: 0.012,
            tree_death_climate: 0.12,
            min_trees: 20,
            canopy_competition_radius: 8.0,

            mountain_cold_offset: 0.30,

            poison_damage: 70.0,
            tree_heavy_graze_threshold: 4,

            social_influence_radius: 10.0,

            // Aquatic ecosystem
            initial_aquatic_plants: 300,
            max_aquatic_plants: 2000,
            aquatic_plant_spawn_rate: 0.05,
            aquatic_plant_max_energy: 45.0,
            aquatic_plant_regrow_rate: 0.4,
            aquatic_eat_shallow_min_swim: 0.25,
            aquatic_eat_deep_min_swim: 0.65,
            water_speed_bonus: 1.4,
            water_stealth_factor: 0.5,

            // Pack hunting
            pack_hunt_bonus: 0.55,
            pack_max_size: 5,

            // Grass & Grazing
            grass_regrow_rate: 0.05,
            grass_max_energy: 20.0,
            grass_graze_max: 4.0,
            sociability_wander_bias: 0.4,

            render_cols: 80,
            render_rows: 24,
        }
    }
}

impl Config {
    /// Climate constants bundled for the [`crate::climate`] helpers.
    pub fn climate_params(&self) -> crate::climate::ClimateParams {
        crate::climate::ClimateParams {
            base: self.climate_base,
            south_amp: self.south_warm_amp,
            season_amp: self.season_amp,
            cycles_per_year: self.cycles_per_year,
        }
    }

    /// Death probability for a creature that has survived `cycles_survived` rolls.
    pub fn death_probability(&self, cycles_survived: u32) -> f64 {
        (self.death_base + self.death_per_cycle * cycles_survived as f64).min(self.death_cap)
    }

    /// Fruit energy scale for the current season.
    /// Autumn (sin≈0, cos<0) is richest; spring (sin≈0, cos>0) is leanest;
    /// summer / winter are intermediate but hot/cold extremes matter for trees.
    pub fn fruit_energy_scale(&self, seasonality: f64, phase_cos: f64) -> f64 {
        // Use a two-component blend: sin drives summer/winter, cos selects transition.
        let is_autumn = phase_cos < 0.0;
        if is_autumn {
            // Autumn–Winter transition: rich → moderate
            self.autumn_energy_bonus + (1.0 - self.autumn_energy_bonus) * (-seasonality + 1.0) * 0.5
        } else {
            // Spring–Summer transition: lean → moderate
            self.spring_energy_penalty + (1.0 - self.spring_energy_penalty) * (seasonality + 1.0) * 0.5
        }
        .clamp(self.spring_energy_penalty, self.autumn_energy_bonus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn death_probability_follows_spec() {
        let c = Config::default();
        assert!((c.death_probability(0) - 0.20).abs() < 1e-9);
        assert!((c.death_probability(1) - 0.24).abs() < 1e-9);
        assert!((c.death_probability(2) - 0.28).abs() < 1e-9);
    }

    #[test]
    fn death_probability_is_capped() {
        let c = Config::default();
        assert!((c.death_probability(1000) - c.death_cap).abs() < 1e-9);
    }

    #[test]
    fn death_is_certain_at_twenty_cycles() {
        let c = Config::default();
        assert!((c.death_probability(20) - 1.0).abs() < 1e-9);
        assert!(c.death_probability(19) < 1.0);
    }

    #[test]
    fn autumn_richer_than_spring() {
        let c = Config::default();
        // Autumn: phase = 3π/2 (cos < 0, sin ≈ -1)
        let autumn = c.fruit_energy_scale(-1.0, -0.1);
        // Spring: phase = π/2 (cos < 0 is summer actually; spring is cos > 0, sin ≈ 0)
        let spring = c.fruit_energy_scale(0.0, 1.0);
        assert!(autumn > spring, "autumn fruit should be richer than spring");
    }
}
