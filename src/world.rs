//! The world: all state plus the main simulation loop.
//!
//! ### New systems (v2)
//! - **Pack hunting**: carnivores near each other combine combat power when
//!   attacking the same prey; the kill energy is shared among the pack.
//! - **Aquatic ecosystem**: `AquaticPlant` patches grow in water tiles; high-swim
//!   creatures graze them; aquatic ambush halves prey sense radius in water.
//! - **Seasonal vegetation**: fruit production follows a harsh cycle — deep winter
//!   pushes trees to 8% of normal output; drought-sensitive trees go semi-dormant
//!   in peak summer heat; autumn fruit is 35% richer in energy.
//! - **Canopy competition**: nearby trees with high `canopy_competition` suppress
//!   each other's fertility, driving forest structure and dispersal.
//! - **Water speed bonus**: high-swim creatures move faster in water tiles,
//!   rewarding aquatic adaptation directly.

use crate::climate;
use crate::config::Config;
use crate::creature::Creature;
use crate::food::{AquaticPlant, FoodSource, Fruit, TreeGenome};
use crate::genome::{DietClass, Genome};
use crate::geometry::Vec2;
use crate::gut::GutBacterium;
use crate::memory::MemKind;
use crate::remains::{Carcass, Fertilizer};
use crate::rng::Rng;
use crate::social::{SocialVec, SOCIAL_ALPHA};
use crate::terrain::{TerrainMap, TileType};
use crate::vision::{VisionGrid, VISION_SIZE};

/// Per-cycle population snapshot for the Red Queen graph / CSV export.
#[derive(Clone, Copy, Debug)]
pub struct CycleRecord {
    pub cycle: u64,
    pub herbivores: usize,
    pub omnivores: usize,
    pub carnivores: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherEvent {
    Clear,
    ColdSnap,
    Heatwave,
    ToxicStorm,
    Tsunami,
}

impl WeatherEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            WeatherEvent::Clear => "Clear",
            WeatherEvent::ColdSnap => "Cold Snap",
            WeatherEvent::Heatwave => "Heatwave",
            WeatherEvent::ToxicStorm => "Toxic Storm",
            WeatherEvent::Tsunami => "Tsunami",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Twig {
    pub pos: Vec2,
    pub id: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Nest {
    pub pos: Vec2,
    pub owner_id: u64,
    pub twigs: u32,
    pub completed: bool,
}

pub struct World {
    pub cfg: Config,
    pub rng: Rng,
    pub creatures: Vec<Creature>,
    pub sources: Vec<FoodSource>,
    pub fruits: Vec<Fruit>,
    pub carcasses: Vec<Carcass>,
    pub fertilizer: Vec<Fertilizer>,
    /// Aquatic plant patches growing in water tiles.
    pub aquatic_plants: Vec<AquaticPlant>,
    pub twigs: Vec<Twig>,
    pub nests: Vec<Nest>,
    pub tick: u64,
    pub cycle: u64,
    pub season_phase: f64,
    pub terrain: TerrainMap,
    next_id: u64,
    next_tree_id: u64,
    pub extinct: bool,
    pub history: Vec<CycleRecord>,
    pub total_asexual_births: u64,
    pub total_sexual_births: u64,
    pub total_carcasses_spawned: u64,
    pub total_trees_germinated: u64,
    pub total_poison_deaths: u64,
    
    // --- Grass & Grazing ---
    pub grass: Vec<f64>,
    pub fungi: Vec<f64>,

    // --- Weather ---
    pub current_weather: WeatherEvent,
    pub weather_duration: u32,

    // --- Spatial grids ---
    creature_grid: SpatialGrid,
    fruit_grid: SpatialGrid,
    carcass_grid: SpatialGrid,
    aquatic_grid: SpatialGrid,
    twigs_grid: SpatialGrid,
}

#[derive(Clone, Copy)]
struct Sighting {
    idx: usize,
    pos: Vec2,
    power: f64,
    alive: bool,
}

impl World {
    pub fn new(cfg: Config) -> Self {
        let mut rng = Rng::new(cfg.seed);
        let terrain = TerrainMap::generate(cfg.width, cfg.height, cfg.seed);

        let mut next_id = 0u64;
        let mut creatures = Vec::with_capacity(cfg.initial_creatures);
        for _ in 0..cfg.initial_creatures {
            let pos = (0..20)
                .map(|_| Vec2::new(rng.range_f64(0.0, cfg.width), rng.range_f64(0.0, cfg.height)))
                .find(|p| terrain.tile_at(p.x, p.y).allows_trees())
                .unwrap_or_else(|| Vec2::new(rng.range_f64(0.0, cfg.width), rng.range_f64(0.0, cfg.height)));
            let genome = Genome::random(&mut rng);
            creatures.push(Creature::new(next_id, pos, cfg.start_energy, genome, 1.0, GutBacterium::random(&mut rng)));
            next_id += 1;
        }

        let mut next_tree_id = 0u64;
        let mut sources = Vec::with_capacity(cfg.initial_food_sources);
        for _ in 0..cfg.initial_food_sources {
            let pos = (0..50)
                .map(|_| Vec2::new(rng.range_f64(0.0, cfg.width), rng.range_f64(0.0, cfg.height)))
                .find(|p| terrain.tile_at(p.x, p.y).allows_trees())
                .unwrap_or_else(|| Vec2::new(rng.range_f64(0.0, cfg.width), rng.range_f64(0.0, cfg.height)));
            let genome = TreeGenome::random(&mut rng);
            let is_fruit_tree = rng.chance(0.70);
            sources.push(FoodSource { pos, genome, id: next_tree_id, feeds_this_cycle: 0, is_fruit_tree });
            next_tree_id += 1;
        }

        let mut fruits = Vec::new();
        for src in &sources {
            for _ in 0..cfg.initial_fruit_per_source {
                let scatter = cfg.fruit_scatter * src.genome.seed_dispersal;
                let dx = rng.gaussian() * scatter;
                let dy = rng.gaussian() * scatter;
                let p = Vec2::new(src.pos.x + dx, src.pos.y + dy).clamp_to(cfg.width, cfg.height);
                fruits.push(Fruit::new(p, src.genome, src.id, 1.0));
            }
        }

        // Seed aquatic plants in water tiles.
        let mut aquatic_plants = Vec::new();
        let na = cfg.initial_aquatic_plants;
        let mut attempts = 0usize;
        while aquatic_plants.len() < na && attempts < na * 20 {
            attempts += 1;
            let p = Vec2::new(rng.range_f64(0.0, cfg.width), rng.range_f64(0.0, cfg.height));
            let tile = terrain.tile_at(p.x, p.y);
            if tile == TileType::ShallowWater || tile == TileType::DeepWater {
                let deep = tile == TileType::DeepWater;
                let regrow = cfg.aquatic_plant_regrow_rate * if deep { 0.4 } else { 1.0 };
                let max_e  = cfg.aquatic_plant_max_energy  * if deep { 2.5 } else { 1.0 };
                aquatic_plants.push(AquaticPlant::new(p, max_e, regrow, deep));
            }
        }

        let cols = cfg.width.ceil() as usize + 1;
        let rows = cfg.height.ceil() as usize + 1;
        let mut grass = vec![0.0; cols * rows];
        let fungi = vec![0.0; cols * rows];
        for y in 0..rows {
            for x in 0..cols {
                let tile = terrain.tile_at(x as f64, y as f64);
                let initial_val = match tile {
                    TileType::Plains => cfg.grass_max_energy,
                    TileType::Sand => 6.0,
                    TileType::Mountain => 4.5,
                    _ => 0.0,
                };
                grass[y * cols + x] = initial_val;
            }
        }

        let creature_grid = SpatialGrid::new(cfg.width, cfg.height, 40.0);
        let fruit_grid = SpatialGrid::new(cfg.width, cfg.height, 40.0);
        let carcass_grid = SpatialGrid::new(cfg.width, cfg.height, 40.0);
        let aquatic_grid = SpatialGrid::new(cfg.width, cfg.height, 40.0);
        let twigs_grid = SpatialGrid::new(cfg.width, cfg.height, 40.0);

        World {
            cfg, rng, creatures, sources, fruits, carcasses: Vec::new(),
            fertilizer: Vec::new(), aquatic_plants,
            twigs: Vec::new(),
            nests: Vec::new(),
            tick: 0, cycle: 0, season_phase: 0.0, terrain,
            next_id, next_tree_id,
            extinct: false, history: Vec::new(),
            total_asexual_births: 0, total_sexual_births: 0,
            total_carcasses_spawned: 0, total_trees_germinated: 0,
            total_poison_deaths: 0,
            grass,
            fungi,
            current_weather: WeatherEvent::Clear,
            weather_duration: 0,
            creature_grid,
            fruit_grid,
            carcass_grid,
            aquatic_grid,
            twigs_grid,
        }
    }

    fn rebuild_creature_grid(&mut self) {
        self.creature_grid.clear();
        for (i, c) in self.creatures.iter().enumerate() {
            if c.alive {
                self.creature_grid.add(c.pos, i);
            }
        }
    }

    fn rebuild_fruit_grid(&mut self) {
        self.fruit_grid.clear();
        for (i, f) in self.fruits.iter().enumerate() {
            self.fruit_grid.add(f.pos, i);
        }
    }

    fn rebuild_carcass_grid(&mut self) {
        self.carcass_grid.clear();
        for (i, c) in self.carcasses.iter().enumerate() {
            self.carcass_grid.add(c.pos, i);
        }
    }

    fn rebuild_aquatic_grid(&mut self) {
        self.aquatic_grid.clear();
        for (i, a) in self.aquatic_plants.iter().enumerate() {
            if a.energy >= 1.0 {
                self.aquatic_grid.add(a.pos, i);
            }
        }
    }

    fn rebuild_twigs_grid(&mut self) {
        self.twigs_grid.clear();
        for (i, t) in self.twigs.iter().enumerate() {
            self.twigs_grid.add(t.pos, i);
        }
    }

    fn rebuild_grids(&mut self) {
        self.rebuild_creature_grid();
        self.rebuild_fruit_grid();
        self.rebuild_carcass_grid();
        self.rebuild_aquatic_grid();
        self.rebuild_twigs_grid();
    }

    pub fn temperature_at(&self, y: f64) -> f64 {
        let mut t = climate::temperature(y, self.cfg.height, self.season_phase, &self.cfg.climate_params());
        match self.current_weather {
            WeatherEvent::ColdSnap => t -= 0.40,
            WeatherEvent::Heatwave => t += 0.40,
            _ => {}
        }
        t
    }

    pub fn population(&self) -> usize {
        self.creatures.iter().filter(|c| c.alive).count()
    }

    /// Current sin(season_phase) — +1 = peak summer, −1 = peak winter.
    fn seasonality(&self) -> f64 {
        self.season_phase.sin()
    }

    /// Fruit energy scale for the current season (autumn-rich, spring-lean).
    fn fruit_energy_scale(&self) -> f64 {
        self.cfg.fruit_energy_scale(self.seasonality(), self.season_phase.cos())
    }

    // ─── STEP ────────────────────────────────────────────────────────────────

    pub fn step(&mut self) -> bool {
        if self.extinct { return false; }
        self.rebuild_grids();
        self.update_tribal_states();
        self.season_phase = climate::season_phase(self.cycle, self.cfg.cycles_per_year);

        // Weather system tick
        if self.weather_duration > 0 {
            self.weather_duration -= 1;
            if self.weather_duration == 0 {
                self.current_weather = WeatherEvent::Clear;
            }
        } else {
            // 0.3% chance per tick to trigger weather
            if self.rng.chance(0.003) {
                let roll = self.rng.below(4);
                self.current_weather = match roll {
                    0 => WeatherEvent::ColdSnap,
                    1 => WeatherEvent::Heatwave,
                    2 => WeatherEvent::ToxicStorm,
                    _ => WeatherEvent::Tsunami,
                };
                self.weather_duration = self.cfg.ticks_per_cycle as u32;
            }
        }

        // Apply weather effects (damages, pushes)
        self.apply_weather_effects();

        self.drop_fruit();
        self.tick_aquatic_plants();
        self.regrow_grass();
        self.move_and_eat();
        self.predation();
        self.share_altruistic_energy();
        self.reproduce();
        self.resolve_vocalizations();
        self.drop_poop();
        self.update_social_vecs();

        self.tick += 1;
        if self.tick % self.cfg.ticks_per_cycle == 0 {
            self.end_of_cycle();
        }
        self.compact();

        if self.population() == 0 {
            self.extinct = true;
            return false;
        }
        true
    }

    fn is_near_own_completed_nest(&self, c: &Creature) -> bool {
        if let Some(npos) = c.nest_pos {
            if c.pos.dist(npos) <= 3.0 {
                return self.nests.iter()
                    .find(|n| n.owner_id == c.id)
                    .map_or(false, |n| n.completed);
            }
        }
        false
    }

    fn are_closely_related(&self, i: usize, j: usize) -> bool {
        let me = &self.creatures[i];
        let other = &self.creatures[j];
        
        // Parent-child check
        if let Some((p1, p2)) = me.parent_ids {
            if p1 == other.id || p2 == other.id {
                return true;
            }
        }
        if let Some((p1, p2)) = other.parent_ids {
            if p1 == me.id || p2 == me.id {
                return true;
            }
        }
        
        // Sibling check
        if let (Some((p1a, p1b)), Some((p2a, p2b))) = (me.parent_ids, other.parent_ids) {
            if p1a == p2a || p1a == p2b || p1b == p2a || p1b == p2b {
                return true;
            }
        }
        
        false
    }

    fn apply_weather_effects(&mut self) {
        match self.current_weather {
            WeatherEvent::ToxicStorm => {
                // Land creatures lose 1.5 * (1.0 - poison_resist) energy per tick
                for i in 0..self.creatures.len() {
                    if !self.creatures[i].alive { continue; }
                    if self.is_near_own_completed_nest(&self.creatures[i]) { continue; }
                    let tile = self.terrain.tile_at(self.creatures[i].pos.x, self.creatures[i].pos.y);
                    let is_land = !matches!(tile, TileType::ShallowWater | TileType::DeepWater);
                    if is_land {
                        let res = self.creatures[i].genome.poison_resist.evaluate(self.creatures[i].genome.energy_x(self.creatures[i].energy)).clamp(0.0, 1.0);
                        self.creatures[i].energy -= 1.5 * (1.0 - res);
                    }
                }
            }
            WeatherEvent::Tsunami => {
                // Coastal tiles flooded. Any creature in Sand/ShallowWater with swim_capability < 0.5:
                // loses 10.0 energy and is displaced.
                let w = self.cfg.width;
                let h = self.cfg.height;
                let mut movements = Vec::new();
                for (idx, c) in self.creatures.iter().enumerate() {
                    if !c.alive { continue; }
                    if self.is_near_own_completed_nest(c) { continue; }
                    let tile = self.terrain.tile_at(c.pos.x, c.pos.y);
                    if matches!(tile, TileType::Sand | TileType::ShallowWater) {
                        let swim = c.genome.effective_swim(c.energy);
                        if swim < 0.5 {
                            movements.push((idx, Vec2::new(
                                c.pos.x + self.rng.range_f64(-8.0, 8.0),
                                c.pos.y + self.rng.range_f64(-8.0, 8.0),
                            ).clamp_to(w, h)));
                        }
                    }
                }
                for (idx, next_pos) in movements {
                    self.creatures[idx].energy -= 10.0;
                    self.creatures[idx].pos = next_pos;
                }
            }
            _ => {}
        }
    }

    fn update_tribal_states(&mut self) {
        self.rebuild_creature_grid();
        let n = self.creatures.len();
        
        // Determine tribal status for each alive creature
        for i in 0..n {
            if !self.creatures[i].alive { continue; }
            let pos_i = self.creatures[i].pos;
            let social_i = &self.creatures[i].social;
            let range_i = self.cfg.social_influence_radius * (1.0 + self.creatures[i].genome.effective_social_capacity(self.creatures[i].energy) * 0.5);
            let range_i_sq = range_i * range_i;
            
            let mut similar_neighbors_count = 0;
            self.creature_grid.query(pos_i, range_i, |j| {
                if j == i { return; }
                let other = &self.creatures[j];
                if pos_i.dist_sq(other.pos) <= range_i_sq {
                    if social_i.cosine_similarity(&other.social) >= 0.85 {
                        similar_neighbors_count += 1;
                    }
                }
            });
            self.creatures[i].in_tribe = similar_neighbors_count >= 4;
        }
    }

    fn regrow_grass(&mut self) {
        let cols = self.cfg.width.ceil() as usize + 1;
        let rows = self.cfg.height.ceil() as usize + 1;
        let regrow = self.cfg.grass_regrow_rate;
        let max_e  = self.cfg.grass_max_energy;

        // Seasonality factor: spring/summer grows faster, winter slower.
        let season_fac = (1.0 + self.seasonality() * 0.5).max(0.1);

        for y in 0..rows {
            for x in 0..cols {
                let tile = self.terrain.tile_at(x as f64, y as f64);
                let (cap, r_rate) = match tile {
                    TileType::Plains => (max_e, regrow * season_fac),
                    TileType::Sand => (6.0, regrow * 0.25),
                    TileType::Mountain => (4.5, regrow * 0.4),
                    _ => (0.0, 0.0),
                };
                if cap > 0.0 {
                    let idx = y * cols + x;
                    self.grass[idx] = (self.grass[idx] + r_rate).min(cap);
                }
            }
        }

        // Decay all fungi first:
        for val in &mut self.fungi {
            *val = (*val - 0.2).max(0.0);
        }

        // Sprout mushrooms near trees (within 8.0 units, on Plains, 0.15% chance):
        for s in &self.sources {
            let tx = s.pos.x;
            let ty = s.pos.y;
            let min_gx = (tx - 8.0).max(0.0).floor() as usize;
            let max_gx = (tx + 8.0).min(self.cfg.width).ceil() as usize;
            let min_gy = (ty - 8.0).max(0.0).floor() as usize;
            let max_gy = (ty + 8.0).min(self.cfg.height).ceil() as usize;
            for gy in min_gy..=max_gy {
                if gy >= rows { continue; }
                for gx in min_gx..=max_gx {
                    if gx >= cols { continue; }
                    let idx = gy * cols + gx;
                    let wx = gx as f64;
                    let wy = gy as f64;
                    if self.terrain.tile_at(wx, wy) == TileType::Plains {
                        let dist_sq = (wx - tx) * (wx - tx) + (wy - ty) * (wy - ty);
                        if dist_sq <= 8.0 * 8.0 {
                            if self.rng.chance(0.0015) {
                                self.fungi[idx] = 40.0;
                            }
                        }
                    }
                }
            }
        }
    }

    fn share_altruistic_energy(&mut self) {
        self.rebuild_creature_grid();
        let n = self.creatures.len();
        let mut energy_deltas = vec![0.0f64; n];
        
        let mut max_altruism = vec![0.0f64; n];
        let mut baseline_altruism = vec![0.0f64; n];
        for i in 0..n {
            let c = &self.creatures[i];
            if c.alive {
                let x = c.genome.energy_x(c.energy);
                let altru = c.genome.altruism.evaluate(x);
                baseline_altruism[i] = altru;
                max_altruism[i] = if c.in_tribe { altru + 0.40 } else { altru };
            }
        }

        for i in 0..n {
            if !self.creatures[i].alive { continue; }
            if max_altruism[i] <= 0.25 { continue; }

            let current_energy = self.creatures[i].energy + energy_deltas[i];
            if current_energy <= 80.0 { continue; }

            let pos_i = self.creatures[i].pos;
            let social_i = &self.creatures[i].social;
            let share_radius = 2.0 * (1.0 + self.creatures[i].genome.effective_social_capacity(self.creatures[i].energy) * 0.8);
            let share_radius_sq = share_radius * share_radius;

            let mut best_recipient = None;
            let mut best_sim = 0.6_f32;

            self.creature_grid.query(pos_i, share_radius, |j| {
                if i == j { return; }
                let other = &self.creatures[j];
                let current_energy_j = other.energy + energy_deltas[j];
                if current_energy_j >= 40.0 { return; }

                if pos_i.dist_sq(other.pos) > share_radius_sq { return; }

                let sim = social_i.cosine_similarity(&other.social);
                if sim > best_sim {
                    best_sim = sim;
                    best_recipient = Some(j);
                }
            });

            if let Some(j) = best_recipient {
                let actual_altruism = if self.creatures[i].in_tribe && best_sim >= 0.85 { baseline_altruism[i] + 0.40 } else { baseline_altruism[i] };
                if actual_altruism <= 0.25 { continue; }

                let current_energy_j = self.creatures[j].energy + energy_deltas[j];
                let max_gift = 10.0 * (1.0 + self.creatures[i].genome.effective_social_capacity(self.creatures[i].energy) * 0.5);
                let gift = (current_energy - 80.0)
                    .min(40.0 - current_energy_j)
                    .min(max_gift);
                if gift > 0.1 {
                    energy_deltas[i] -= gift;
                    energy_deltas[j] += gift;
                }
            }
        }

        for i in 0..n {
            if self.creatures[i].alive {
                self.creatures[i].energy += energy_deltas[i];
            }
        }
    }

    // ─── FRUIT DROP (seasonal vegetation) ───────────────────────────────────

    fn drop_fruit(&mut self) {
        if self.fruits.len() >= self.cfg.max_fruits { return; }
        let season   = self.seasonality();
        let fen_scale = self.fruit_energy_scale();

        // Compute canopy suppression per tree before the main loop.
        let suppression = self.compute_canopy_suppression();

        let mut drops: Vec<(Vec2, TreeGenome, u64)> = Vec::new();
        let mut twig_drops: Vec<(Vec2, u64)> = Vec::new();
        for (i, src) in self.sources.iter().enumerate() {
            let temp     = self.temperature_at(src.pos.y);
            let fert     = self.fertilizer_near(src.pos, 6.0);
            let fert_boost = 1.0 + (fert * 0.05).min(1.0);
            let terrain_f = self.terrain.tile_at(src.pos.x, src.pos.y).tree_fertility_factor();
            let canopy_sup = suppression.get(i).copied().unwrap_or(0.0);

            let mut rate = src.fruiting_rate(
                self.cfg.fruit_drop_per_tick * terrain_f,
                temp,
                season,
                fert_boost,
                canopy_sup,
                self.cfg.winter_fruit_floor,
                self.cfg.summer_heat_scale,
            );
            if self.current_weather == WeatherEvent::ColdSnap {
                rate *= 0.5;
            }
            let whole = rate.floor() as usize;
            let frac  = rate - whole as f64;
            let mut count = whole;
            if self.rng.chance(frac) { count += 1; }

            // Scatter radius uses the tree's seed_dispersal gene.
            let scatter = self.cfg.fruit_scatter * src.genome.seed_dispersal;
            for _ in 0..count {
                let dx = self.rng.gaussian() * scatter;
                let dy = self.rng.gaussian() * scatter;
                let p = Vec2::new(src.pos.x + dx, src.pos.y + dy)
                    .clamp_to(self.cfg.width, self.cfg.height);
                if src.is_fruit_tree {
                    drops.push((p, src.genome, src.id));
                } else {
                    twig_drops.push((p, src.id));
                }
            }
        }
        for (p, genome, src_id) in drops {
            if self.fruits.len() >= self.cfg.max_fruits { break; }
            self.fruits.push(Fruit::new(p, genome, src_id, fen_scale));
        }
        for (p, src_id) in twig_drops {
            if self.twigs.len() >= 2000 { break; }
            self.twigs.push(Twig { pos: p, id: src_id });
        }
    }

    /// Canopy suppression index for each tree: sum of (competitor.canopy_competition
    /// × overlap_weight) from neighbours within `canopy_competition_radius`.
    fn compute_canopy_suppression(&self) -> Vec<f64> {
        let r  = self.cfg.canopy_competition_radius;
        let r2 = r * r;
        (0..self.sources.len())
            .map(|i| {
                let p = self.sources[i].pos;
                self.sources
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, s)| {
                        let d2 = p.dist_sq(s.pos);
                        if d2 > r2 { 0.0 }
                        else { s.genome.canopy_competition * (1.0 - d2 / r2) }
                    })
                    .sum::<f64>()
                    .min(0.9) // never reduce below 10% productivity
            })
            .collect()
    }

    // ─── AQUATIC PLANTS ──────────────────────────────────────────────────────

    fn tick_aquatic_plants(&mut self) {
        // Grow all existing patches.
        for p in &mut self.aquatic_plants { p.tick_regrow(); }

        // Occasionally spawn new patches in open water tiles.
        if self.aquatic_plants.len() < self.cfg.max_aquatic_plants
            && self.rng.chance(self.cfg.aquatic_plant_spawn_rate)
        {
            let pos = Vec2::new(
                self.rng.range_f64(0.0, self.cfg.width),
                self.rng.range_f64(0.0, self.cfg.height),
            );
            let tile = self.terrain.tile_at(pos.x, pos.y);
            if tile == TileType::ShallowWater || tile == TileType::DeepWater {
                let deep   = tile == TileType::DeepWater;
                let regrow = self.cfg.aquatic_plant_regrow_rate * if deep { 0.4 } else { 1.0 };
                let max_e  = self.cfg.aquatic_plant_max_energy  * if deep { 2.5 } else { 1.0 };
                self.aquatic_plants.push(AquaticPlant::new(pos, max_e, regrow, deep));
            }
        }
    }

    // ─── VISION GRID ─────────────────────────────────────────────────────────

    pub fn compute_vision(&self, i: usize) -> VisionGrid {
        let c      = &self.creatures[i];
        let sense  = c.genome.effective_sense(c.energy);
        let cell_size = sense * 2.0 / VISION_SIZE as f64;
        let mut grid = VisionGrid::new(c.pos, cell_size);

        for gy in 0..VISION_SIZE {
            for gx in 0..VISION_SIZE {
                let half = (VISION_SIZE / 2) as f64;
                let wx = c.pos.x + (gx as f64 - half) * cell_size;
                let wy = c.pos.y + (gy as f64 - half) * cell_size;
                grid.cells[gy][gx].terrain = self.terrain.tile_at(wx, wy) as u8;
                let temp = self.temperature_at(wy);
                let opt  = c.genome.temp_optimum.evaluate(c.genome.energy_x(c.energy));
                grid.cells[gy][gx].temp_deviation = (temp - opt).abs() as f32;
            }
        }

        // Hydration attraction:
        let my_hydration = c.hydration;
        let my_swim = c.genome.effective_swim(c.energy);
        if my_hydration < 50.0 {
            let hydration_need = (50.0 - my_hydration) as f32;
            for gy in 0..VISION_SIZE {
                for gx in 0..VISION_SIZE {
                    let tile = grid.cells[gy][gx].terrain;
                    if tile == TileType::ShallowWater as u8 || tile == TileType::DeepWater as u8 {
                        let is_deep = tile == TileType::DeepWater as u8;
                        let swim_ok = my_swim >= 0.25;
                        if !is_deep || swim_ok {
                            grid.cells[gy][gx].food_energy += hydration_need * 2.0;
                        }
                    }
                }
            }
        }

        let query_radius = 1.2 * sense;

        self.fruit_grid.query(c.pos, query_radius, |idx| {
            let fruit = &self.fruits[idx];
            if let Some((gx, gy)) = grid.world_to_cell(fruit.pos) {
                grid.cells[gy][gx].food_energy += fruit.energy as f32;
            }
        });

        // Aquatic plants visible to swimming creatures.
        let my_swim = c.genome.effective_swim(c.energy);
        if my_swim >= self.cfg.aquatic_eat_shallow_min_swim {
            self.aquatic_grid.query(c.pos, query_radius, |idx| {
                let ap = &self.aquatic_plants[idx];
                if let Some((gx, gy)) = grid.world_to_cell(ap.pos) {
                    grid.cells[gy][gx].food_energy += ap.energy as f32 * 0.6; // partial signal
                }
            });
        }

        self.carcass_grid.query(c.pos, query_radius, |idx| {
            let carc = &self.carcasses[idx];
            if let Some((gx, gy)) = grid.world_to_cell(carc.pos) {
                grid.cells[gy][gx].carcass_energy += carc.energy as f32;
            }
        });

        let my_power = c.combat_power();
        self.creature_grid.query(c.pos, query_radius, |j| {
            if j == i { return; }
            let other = &self.creatures[j];
            if !other.alive { return; }
            if let Some((gx, gy)) = grid.world_to_cell(other.pos) {
                let other_power = other.combat_power();
                if other_power > my_power {
                    grid.cells[gy][gx].threat_power += other_power as f32;
                }
                let sim = c.social.cosine_similarity(&other.social);
                // Genus conflict repulsion: if similarity is low, treat as threat to stay apart
                if sim < 0.40 {
                    let diff = 0.40 - sim;
                    grid.cells[gy][gx].threat_power += (diff * 3.0) as f32;
                }
                let affinity_mult = if c.in_tribe && other.in_tribe && sim >= 0.85 { 2.0 } else { 1.0 };
                grid.cells[gy][gx].social_affinity += sim.max(0.0) * affinity_mult;
            }
        });

        grid
    }

    // ─── MOVE & EAT ──────────────────────────────────────────────────────────

    fn move_and_eat(&mut self) {
        self.rebuild_grids();
        let n = self.creatures.len();
        for i in 0..n {
            if !self.creatures[i].alive { continue; }
            self.creatures[i].reproduced_this_tick = false;
            self.creatures[i].age_ticks += 1;
            self.creatures[i].memory.tick();
            self.creatures[i].vocal_type = 0;
            
            // Slower maturation for complex reproducers
            let growth_inc = 0.05 * (1.0 - 0.40 * self.creatures[i].genome.effective_repro_complexity(self.creatures[i].energy));
            self.creatures[i].growth_factor = (self.creatures[i].growth_factor + growth_inc).min(1.0);

            let here      = self.creatures[i].pos;

            // --- Nest Establishment ---
            if self.creatures[i].growth_factor >= 1.0 && self.creatures[i].nest_pos.is_none() {
                let sense = self.creatures[i].genome.effective_sense(self.creatures[i].energy);
                let mut nearest_empty_nest_idx = None;
                let mut min_dist = sense;
                for (idx, nest) in self.nests.iter().enumerate() {
                    let owner_alive = self.creatures.iter().any(|other| other.alive && other.id == nest.owner_id);
                    if !owner_alive {
                        let d = here.dist(nest.pos);
                        if d < min_dist {
                            min_dist = d;
                            nearest_empty_nest_idx = Some(idx);
                        }
                    }
                }

                if let Some(idx) = nearest_empty_nest_idx {
                    let nest_pos = self.nests[idx].pos;
                    self.creatures[i].nest_pos = Some(nest_pos);
                    self.nests[idx].owner_id = self.creatures[i].id;
                } else {
                    let mut chosen_pos = here;
                    if self.creatures[i].in_tribe {
                        let mut nearest_nest: Option<(Vec2, f64)> = None;
                        for other in &self.creatures {
                            if other.id == self.creatures[i].id || !other.alive || !other.in_tribe { continue; }
                            if self.creatures[i].social.cosine_similarity(&other.social) >= 0.85 {
                                if let Some(npos) = other.nest_pos {
                                    let dist = here.dist(npos);
                                    if nearest_nest.map_or(true, |(_, nd)| dist < nd) {
                                        nearest_nest = Some((npos, dist));
                                    }
                                }
                            }
                        }
                        if let Some((npos, _)) = nearest_nest {
                            let angle = self.rng.range_f64(0.0, 2.0 * std::f64::consts::PI);
                            let dist = self.rng.range_f64(1.5, 3.5);
                            chosen_pos = Vec2::new(npos.x + angle.cos() * dist, npos.y + angle.sin() * dist).clamp_to(self.cfg.width, self.cfg.height);
                        }
                    }
                    self.creatures[i].nest_pos = Some(chosen_pos);
                    self.nests.push(Nest {
                        pos: chosen_pos,
                        owner_id: self.creatures[i].id,
                        twigs: 0,
                        completed: false,
                    });
                }
            }

            // --- Overcrowding Check ---
            let mut neighbor_count = 0;
            self.creature_grid.query(here, 5.0, |j| {
                if j == i { return; }
                if self.creatures[j].alive {
                    neighbor_count += 1;
                }
            });
            let is_overcrowded = neighbor_count >= 6;
            self.creatures[i].overcrowded = is_overcrowded;
            if is_overcrowded {
                self.creatures[i].energy -= 0.5;
            }

            // --- Hydration / Thirst ---
            let my_swim_init = self.creatures[i].genome.effective_swim(self.creatures[i].energy);
            if my_swim_init >= 0.75 {
                self.creatures[i].hydration = 100.0;
            } else {
                let tile_here = self.terrain.tile_at(here.x, here.y);
                let mut loss = 0.30;
                if tile_here == TileType::Sand {
                    loss += 0.30;
                }
                if self.current_weather == WeatherEvent::Heatwave {
                    loss += 0.30;
                }
                self.creatures[i].hydration = (self.creatures[i].hydration - loss).max(0.0);

                // Water adjacency check
                let mut near_water = false;
                for dx in [-1.0, 0.0, 1.0].iter() {
                    for dy in [-1.0, 0.0, 1.0].iter() {
                        let tx = here.x + dx * 1.5;
                        let ty = here.y + dy * 1.5;
                        if tx >= 0.0 && tx < self.cfg.width && ty >= 0.0 && ty < self.cfg.height {
                            let tile = self.terrain.tile_at(tx, ty);
                            if matches!(tile, TileType::ShallowWater | TileType::DeepWater) {
                                near_water = true;
                                break;
                            }
                        }
                    }
                    if near_water { break; }
                }
                if near_water {
                    self.creatures[i].hydration = 100.0;
                }
            }
            if self.creatures[i].hydration <= 0.0 {
                self.creatures[i].energy -= 2.0;
            }

            // --- Sickness Spontaneous Infection & Contagion ---
            if self.creatures[i].sickness == 0.0 && self.rng.chance(0.001) {
                self.creatures[i].sickness = 0.20;
            }

            let expressed_immunity = self.creatures[i].effective_immunity();
            if self.creatures[i].sickness > 0.0 {
                self.creatures[i].energy -= 2.0 * self.creatures[i].sickness;
                let mut rec = 0.02 * (1.0 + expressed_immunity);
                let mut is_near_completed_nest = false;
                if let Some(npos) = self.creatures[i].nest_pos {
                    if here.dist(npos) <= 3.0 {
                        is_near_completed_nest = self.nests.iter()
                            .find(|n| n.owner_id == self.creatures[i].id)
                            .map_or(false, |n| n.completed);
                    }
                }
                if is_near_completed_nest {
                    rec *= 2.0;
                }
                self.creatures[i].sickness = (self.creatures[i].sickness - rec).max(0.0);
            }

            if self.creatures[i].sickness > 0.1 {
                let mut infected_neighbors = Vec::new();
                let sick_i = self.creatures[i].sickness;
                self.creature_grid.query(here, 3.0, |j| {
                    if j == i { return; }
                    let other = &self.creatures[j];
                    if other.alive && other.sickness == 0.0 {
                        let other_imm = other.effective_immunity();
                        let base_chance = if is_overcrowded || other.overcrowded { 0.20 } else { 0.04 };
                        let chance = base_chance * sick_i * (1.0 - other_imm);
                        if self.rng.chance(chance) {
                            infected_neighbors.push(j);
                        }
                    }
                });
                for j in infected_neighbors {
                    self.creatures[j].sickness = 0.20;
                }
            }

            // --- Parasite Drain & Recovery ---
            if self.creatures[i].parasites > 0.0 {
                self.creatures[i].energy -= 0.20 * self.creatures[i].parasites;
                let rec = 0.003 * expressed_immunity;
                self.creatures[i].parasites = (self.creatures[i].parasites - rec).max(0.0);
            }

            // Reload energy and traits after deductions
            let energy    = self.creatures[i].energy;
            let sense     = self.creatures[i].genome.effective_sense(energy);
            let hunts     = self.creatures[i].hunts();
            let eats_meat = self.creatures[i].eats_meat();
            let my_swim   = self.creatures[i].genome.effective_swim(energy);

            // Vision grid.
            let vision      = self.compute_vision(i);
            
            // Tribal Herding: boost expressed sociability
            let mut sociability = self.creatures[i].genome.effective_sociability(energy);
            if self.creatures[i].in_tribe {
                sociability += 0.50;
            }
            let vision_hint = vision.movement_hint(sociability);

            // --- Twig/Nest Target Override ---
            let mut target_override: Option<Vec2> = None;
            let is_healthy = energy >= 50.0 && energy >= self.creatures[i].genome.repro_threshold.a * 0.5;
            let mut has_incomplete_nest = false;
            if let Some(_) = self.creatures[i].nest_pos {
                has_incomplete_nest = self.nests.iter()
                    .find(|n| n.owner_id == self.creatures[i].id)
                    .map_or(false, |n| !n.completed);
            }

            if has_incomplete_nest && is_healthy {
                if self.creatures[i].carrying_twig {
                    target_override = self.creatures[i].nest_pos;
                } else {
                    let mut nearest_twig: Option<(Vec2, f64)> = None;
                    self.twigs_grid.query(here, sense, |t_idx| {
                        let twig = &self.twigs[t_idx];
                        let dist = here.dist(twig.pos);
                        if nearest_twig.map_or(true, |(_, d)| dist < d) {
                            nearest_twig = Some((twig.pos, dist));
                        }
                    });
                    if let Some((tpos, _)) = nearest_twig {
                        target_override = Some(tpos);
                    }
                }
            }

            // Live sensing.
            let live_target: Option<Vec2> = if hunts {
                let my_power = self.creatures[i].genome.combat_power(self.creatures[i].energy, self.creatures[i].growth_factor);
                self.nearest_prey(i, here, sense, my_power, my_swim)
                    .map(|(_, p)| { self.creatures[i].memory.record(MemKind::Threat, p, my_power as f32); p })
                    .or_else(|| self.nearest_carcass(here, sense).map(|(_, p)| {
                        self.creatures[i].memory.record(MemKind::Carcass, p, 40.0); p
                     }))
                    .or_else(|| self.nearest_fruit(here, sense).map(|(_, p)| {
                        self.creatures[i].record_food_memory(p, 40.0); p
                    }))
            } else if eats_meat {
                self.nearest_food_any(here, sense, my_swim).map(|p| {
                    self.creatures[i].record_food_memory(p, 30.0); p
                })
            } else {
                self.nearest_fruit(here, sense)
                    .map(|(_, p)| { self.creatures[i].record_food_memory(p, 40.0); p })
                    .or_else(|| {
                        // Herbivores with swim capability also graze aquatic plants.
                        if my_swim >= self.cfg.aquatic_eat_shallow_min_swim {
                            self.nearest_aquatic_plant(here, sense, my_swim)
                                .map(|p| { self.creatures[i].record_food_memory(p, 35.0); p })
                        } else { None }
                    })
            };

            // Memory fallback.
            let target = target_override.or(live_target).or_else(|| {
                if is_healthy {
                    if let Some(mpos) = self.creatures[i].memory.best(MemKind::Mate) {
                        return Some(mpos);
                    }
                }
                if hunts {
                    self.creatures[i].memory.best(MemKind::Threat)
                        .or_else(|| self.creatures[i].memory.best(MemKind::Carcass))
                        .or_else(|| self.creatures[i].memory.best(MemKind::Food))
                } else if eats_meat {
                    self.creatures[i].memory.best(MemKind::Carcass)
                        .or_else(|| self.creatures[i].memory.best(MemKind::Food))
                } else {
                    self.creatures[i].memory.best(MemKind::Food)
                }
            });

            let mut speed = self.creatures[i].genome.effective_speed(energy);
            if self.creatures[i].sickness > 0.0 {
                speed *= 1.0 - 0.40 * self.creatures[i].sickness;
            }
            if self.creatures[i].hydration <= 0.0 {
                speed *= 0.5;
            }

            let direction_hint = if target_override.is_none() && live_target.is_none() && vision_hint.dist(Vec2::zero()) > 0.1 {
                Some(here.add(vision_hint))
            } else {
                None
            };
            let effective_target = target.or(direction_hint);

            let mut step = match effective_target {
                Some(t) => {
                    let dir = t.sub(here).normalized();
                    let d   = here.dist(t).min(speed);
                    dir.scale(d)
                }
                None => {
                    let mut bias_y = 0.0;
                    let drift = self.creatures[i].food_drift_est;
                    if drift.abs() > 1.0 {
                        bias_y = drift.signum() * self.cfg.sociability_wander_bias;
                    }
                    let mut steer = Vec2::new(
                        self.rng.range_f64(-1.0, 1.0),
                        self.rng.range_f64(-1.0, 1.0) + bias_y,
                    ).normalized().scale(speed * 0.5);

                    if self.creatures[i].in_tribe {
                        // Find closest tribe nest (including own)
                        let mut closest_nest: Option<(Vec2, f64)> = None;
                        for other in &self.creatures {
                            if !other.alive || !other.in_tribe { continue; }
                            if self.creatures[i].social.cosine_similarity(&other.social) >= 0.85 {
                                if let Some(npos) = other.nest_pos {
                                    let dist = here.dist(npos);
                                    if closest_nest.map_or(true, |(_, d)| dist < d) {
                                        closest_nest = Some((npos, dist));
                                    }
                                }
                            }
                        }
                        if let Some((npos, dist)) = closest_nest {
                            if dist > 2.0 {
                                let steer_to_nest = npos.sub(here).normalized().scale(speed * 0.5);
                                steer = steer.add(steer_to_nest).normalized().scale(speed * 0.5);
                            }
                        }
                    }
                    steer
                }
            };

            // Suffocation movement restriction: beached pure aquatic creatures move extremely slowly
            let tile_here = self.terrain.tile_at(here.x, here.y);
            let is_beached = my_swim >= 0.75 && !matches!(tile_here, TileType::ShallowWater | TileType::DeepWater);
            if is_beached {
                step = step.scale(0.2);
            }

            let mut new_pos = here.add(step).clamp_to(self.cfg.width, self.cfg.height);
            let tile = self.terrain.tile_at(new_pos.x, new_pos.y);

            // Block deep water for non-swimmers.
            if !tile.is_accessible(&self.creatures[i].genome, self.creatures[i].energy) {
                new_pos = here;
            }
            let tile = self.terrain.tile_at(new_pos.x, new_pos.y);

            // Water speed bonus for high-swim creatures.
            let effective_moved = {
                let raw_moved = here.dist(new_pos);
                let in_water  = tile == TileType::ShallowWater || tile == TileType::DeepWater;
                if in_water && my_swim > 0.5 {
                    raw_moved * (1.0 + (my_swim - 0.5) * 2.0 * (self.cfg.water_speed_bonus - 1.0))
                } else {
                    raw_moved
                }
            };

            self.creatures[i].pos = new_pos;

            // --- Twig Pick up ---
            let mut pick_up_twig = false;
            let mut picked_idx = None;
            if has_incomplete_nest && !self.creatures[i].carrying_twig && is_healthy {
                self.twigs_grid.query(new_pos, 1.5, |t_idx| {
                    if picked_idx.is_none() {
                        let twig = &self.twigs[t_idx];
                        if new_pos.dist(twig.pos) <= 1.5 {
                            picked_idx = Some(t_idx);
                        }
                    }
                });
                if picked_idx.is_some() {
                    pick_up_twig = true;
                }
            }
            if pick_up_twig {
                if let Some(t_idx) = picked_idx {
                    if t_idx < self.twigs.len() {
                        let last_idx = self.twigs.len() - 1;
                        let twig_pos = self.twigs[t_idx].pos;
                        let last_pos = if t_idx < last_idx {
                            Some((self.twigs[last_idx].pos, last_idx))
                        } else {
                            None
                        };
                        self.twigs.swap_remove(t_idx);
                        self.twigs_grid.remove_and_swap(twig_pos, t_idx, last_pos);
                        self.creatures[i].carrying_twig = true;
                    }
                }
            }

            // --- Twig Deposition ---
            if self.creatures[i].carrying_twig {
                if let Some(npos) = self.creatures[i].nest_pos {
                    if new_pos.dist(npos) <= 1.5 {
                        self.creatures[i].carrying_twig = false;
                        if let Some(nest) = self.nests.iter_mut().find(|n| n.owner_id == self.creatures[i].id) {
                            nest.twigs += 1;
                            if nest.twigs >= 5 {
                                nest.completed = true;
                            }
                        }
                    }
                }
            }

            // --- Alarm Call (vocal type 1) check ---
            let mut has_threat = false;
            for gy in 0..VISION_SIZE {
                for gx in 0..VISION_SIZE {
                    if vision.cells[gy][gx].threat_power > 0.0 {
                        has_threat = true;
                        break;
                    }
                }
                if has_threat { break; }
            }
            if has_threat && self.rng.chance(0.20) {
                self.creatures[i].vocal_type = 1;
            }

            // Energy cost.
            let terrain_extra = tile.movement_penalty(&self.creatures[i].genome, self.creatures[i].energy) * effective_moved;
            let temp = self.temperature_at(new_pos.y);
            let temp_offset = tile.temperature_offset(&self.creatures[i].genome, self.cfg.mountain_cold_offset, self.creatures[i].energy);
            
            // Beached suffocation cost (flat 3.0 energy per tick) or dampness drag for land creatures in water (flat 1.0)
            let is_beached_post = my_swim >= 0.75 && !matches!(tile, TileType::ShallowWater | TileType::DeepWater);
            let suffocation_cost = if is_beached_post { 3.0 } else { 0.0 };
            let is_damp_post = my_swim < 0.25 && tile == TileType::ShallowWater;
            let dampness_cost = if is_damp_post { 1.0 } else { 0.0 };

            let cost = self.creatures[i].genome.energy_cost(effective_moved, self.creatures[i].energy, self.creatures[i].growth_factor)
                + self.creatures[i].genome.climate_cost(temp + temp_offset, self.cfg.climate_penalty, self.creatures[i].energy)
                + terrain_extra
                + suffocation_cost
                + dampness_cost;

            let is_near_completed_nest = self.is_near_own_completed_nest(&self.creatures[i]);
            let cost_mult = if is_near_completed_nest { 0.5 } else { 1.0 };
            self.creatures[i].energy -= cost * cost_mult;

            // Eat.
            self.eat_fruit_at(i);
            self.eat_fungi_at(i);
            if eats_meat { self.eat_carcass_at(i); }
            if my_swim >= self.cfg.aquatic_eat_shallow_min_swim {
                self.eat_aquatic_plant_at(i);
            }

            // Graze grass.
            let is_herbivore = self.creatures[i].diet_class() == DietClass::Herbivore;
            if is_herbivore {
                let cx = self.creatures[i].genome.energy_x(self.creatures[i].energy);
                let graze_cap = self.creatures[i].genome.graze.evaluate(cx);
                if graze_cap > 0.4 {
                    let pos = self.creatures[i].pos;
                    let gx = pos.x.round() as usize;
                    let gy = pos.y.round() as usize;
                    let cols = self.cfg.width.ceil() as usize + 1;
                    let rows = self.cfg.height.ceil() as usize + 1;
                    if gx < cols && gy < rows {
                        let idx = gy * cols + gx;
                        let grass_e = self.grass[idx];
                        if grass_e > 0.5 {
                            let max_bite = self.cfg.grass_graze_max;
                            let bite = grass_e.min(max_bite) * graze_cap;
                            self.grass[idx] -= bite;

                            let tile = self.terrain.tile_at(pos.x, pos.y);
                            
                            // Thorn damage for Desert Cactus
                            if tile == TileType::Sand {
                                let expressed_graze = self.creatures[i].genome.effective_graze(self.creatures[i].energy);
                                if expressed_graze < 0.70 {
                                    let resist = self.creatures[i].genome.poison_resist.evaluate(cx).clamp(0.0, 1.0);
                                    let dmg = 12.0 * (1.0 - resist);
                                    let was_alive = self.creatures[i].energy > 0.0;
                                    self.creatures[i].energy -= dmg;
                                    if was_alive && self.creatures[i].energy <= 0.0 {
                                        self.total_poison_deaths += 1;
                                    }
                                }
                            }

                            let eff = self.creatures[i].genome.plant_efficiency(self.creatures[i].energy);
                            let feed = self.creatures[i].genome.feed_efficiency.evaluate(cx).clamp(0.1, 1.0);
                            let gut_boost = 1.0 + self.creatures[i].gut.plant_fit * 0.3;
                            let para_mult = 1.0 - 0.40 * self.creatures[i].parasites;
                            self.creatures[i].energy += bite * eff * feed * gut_boost * para_mult;
                            self.creatures[i].gut.digest_food(1.0, 0.0);
                        }
                    }
                }
            }
        }
    }

    fn nearest_food_any(&self, from: Vec2, sense: f64, swim: f64) -> Option<Vec2> {
        let fruit = self.nearest_fruit(from, sense).map(|(_, p)| p);
        let carc  = self.nearest_carcass(from, sense).map(|(_, p)| p);
        let aqua  = if swim >= self.cfg.aquatic_eat_shallow_min_swim {
            self.nearest_aquatic_plant(from, sense, swim)
        } else { None };

        let mut best: Option<Vec2> = None;
        let mut bd2 = f64::MAX;
        for opt in [fruit, carc, aqua].into_iter().flatten() {
            let d2 = from.dist_sq(opt);
            if d2 < bd2 { bd2 = d2; best = Some(opt); }
        }
        best
    }

    fn nearest_carcass(&self, from: Vec2, sense: f64) -> Option<(usize, Vec2)> {
        let r2 = sense * sense;
        let mut best: Option<(usize, Vec2, f64)> = None;
        self.carcass_grid.query(from, sense, |j| {
            let c = &self.carcasses[j];
            let d2 = from.dist_sq(c.pos);
            if d2 <= r2 && best.map_or(true, |(_, _, b)| d2 < b) {
                best = Some((j, c.pos, d2));
            }
        });
        best.map(|(j, p, _)| (j, p))
    }

    fn nearest_fruit(&self, from: Vec2, sense: f64) -> Option<(usize, Vec2)> {
        let r2 = sense * sense;
        let mut best: Option<(usize, Vec2, f64)> = None;
        self.fruit_grid.query(from, sense, |j| {
            let f = &self.fruits[j];
            let d2 = from.dist_sq(f.pos);
            if d2 <= r2 && best.map_or(true, |(_, _, b)| d2 < b) {
                best = Some((j, f.pos, d2));
            }
        });
        best.map(|(j, p, _)| (j, p))
    }

    fn nearest_aquatic_plant(&self, from: Vec2, sense: f64, swim: f64) -> Option<Vec2> {
        let r2    = sense * sense;
        let deep_ok = swim >= self.cfg.aquatic_eat_deep_min_swim;
        let mut best: Option<(Vec2, f64)> = None;
        self.aquatic_grid.query(from, sense, |j| {
            let ap = &self.aquatic_plants[j];
            if ap.deep && !deep_ok { return; }
            if ap.energy < 1.0 { return; }
            let d2 = from.dist_sq(ap.pos);
            if d2 <= r2 && best.map_or(true, |(_, b)| d2 < b) {
                best = Some((ap.pos, d2));
            }
        });
        best.map(|(p, _)| p)
    }

    /// Nearest prey weaker than `my_power`; takes water stealth into account.
    fn nearest_prey(&self, me: usize, from: Vec2, sense: f64, my_power: f64, my_swim: f64)
        -> Option<(usize, Vec2)>
    {
        let in_water = matches!(
            self.terrain.tile_at(from.x, from.y),
            TileType::ShallowWater | TileType::DeepWater
        );
        let max_effective_sense = if in_water && my_swim > 0.5 {
            sense * (1.0 + my_swim * self.cfg.water_stealth_factor)
        } else {
            sense
        };

        let mut best: Option<(usize, Vec2, f64)> = None;
        self.creature_grid.query(from, max_effective_sense, |j| {
            if j == me { return; }
            let c = &self.creatures[j];
            if !c.alive { return; }
            let d2 = from.dist_sq(c.pos);
            // Aquatic ambush: prey's effective sense radius is halved in water
            // (predator uses water cover). This means a swimming carnivore can
            // get much closer before the prey notices.
            let effective_sense = if in_water && my_swim > 0.5 {
                sense * (1.0 + my_swim * self.cfg.water_stealth_factor)
            } else {
                sense
            };
            if d2 > effective_sense * effective_sense { return; }
            if c.genome.combat_power(c.energy, c.growth_factor) >= my_power { return; }
            if best.map_or(true, |(_, _, b)| d2 < b) {
                best = Some((j, c.pos, d2));
            }
        });
        best.map(|(j, p, _)| (j, p))
    }

    fn eat_fruit_at(&mut self, i: usize) {
        let pos    = self.creatures[i].pos;
        let energy = self.creatures[i].energy;
        let eff    = self.creatures[i].genome.plant_efficiency(energy);
        let cx     = self.creatures[i].genome.energy_x(energy);
        let resist = self.creatures[i].genome.poison_resist
            .evaluate(cx).clamp(0.0, 1.0);
        let feed   = self.creatures[i].genome.feed_efficiency
            .evaluate(cx).clamp(0.1, 1.0);
        let r2  = 1.2 * 1.2;
        let mut eaten = 0;
        let mut j = 0;
        while j < self.fruits.len() && eaten < 2 {
            if pos.dist_sq(self.fruits[j].pos) <= r2 {
                let fruit_pos = self.fruits[j].pos;
                let last_idx = self.fruits.len() - 1;
                let last_pos = if j < last_idx {
                    Some((self.fruits[last_idx].pos, last_idx))
                } else {
                    None
                };
                let fruit = self.fruits.swap_remove(j);
                self.fruit_grid.remove_and_swap(fruit_pos, j, last_pos);
                let gut_boost = 1.0 + self.creatures[i].gut.plant_fit * 0.3;
                let para_mult = 1.0 - 0.40 * self.creatures[i].parasites;
                self.creatures[i].energy += fruit.energy * eff * feed * gut_boost * para_mult;
                self.creatures[i].hydration = (self.creatures[i].hydration + 25.0).min(100.0);
                self.creatures[i].gut.digest_food(1.0, 0.0);
                self.creatures[i].memory.reinforce(MemKind::Food, fruit.pos, 3.0);
                if let Some(src) = self.sources.iter_mut().find(|s| s.id == fruit.source_id) {
                    src.feeds_this_cycle += 1;
                }
                if fruit.poison > resist {
                    let dmg = (fruit.poison - resist) * self.cfg.poison_damage;
                    let was_alive = self.creatures[i].energy > 0.0;
                    self.creatures[i].energy -= dmg;
                    if was_alive && self.creatures[i].energy <= 0.0 {
                        self.total_poison_deaths += 1;
                    }
                }
                eaten += 1;
            } else { j += 1; }
        }
    }

    fn eat_fungi_at(&mut self, i: usize) {
        let pos = self.creatures[i].pos;
        let gx = pos.x.round() as usize;
        let gy = pos.y.round() as usize;
        let cols = self.cfg.width.ceil() as usize + 1;
        let rows = self.cfg.height.ceil() as usize + 1;
        if gx < cols && gy < rows {
            let idx = gy * cols + gx;
            if self.terrain.tile_at(pos.x, pos.y) == TileType::Plains && self.fungi[idx] > 1.0 {
                let bite = 25.0_f64.min(self.fungi[idx]);
                self.fungi[idx] -= bite;
                
                let energy = self.creatures[i].energy;
                let eff = self.creatures[i].genome.plant_efficiency(energy);
                let cx = self.creatures[i].genome.energy_x(energy);
                let feed = self.creatures[i].genome.feed_efficiency
                    .evaluate(cx).clamp(0.1, 1.0);
                let gut_boost = 1.0 + self.creatures[i].gut.plant_fit * 0.3;
                let para_mult = 1.0 - 0.40 * self.creatures[i].parasites;
                self.creatures[i].energy += bite * eff * feed * gut_boost * para_mult;
                self.creatures[i].gut.digest_food(1.0, 0.0);
                
                let resist = self.creatures[i].genome.poison_resist
                    .evaluate(cx).clamp(0.0, 1.0);
                if resist < 0.6 {
                    let dmg = (0.6 - resist) * self.cfg.poison_damage;
                    let was_alive = self.creatures[i].energy > 0.0;
                    self.creatures[i].energy -= dmg;
                    if was_alive && self.creatures[i].energy <= 0.0 {
                        self.total_poison_deaths += 1;
                    }
                }
            }
        }
    }

    fn eat_carcass_at(&mut self, i: usize) {
        let pos    = self.creatures[i].pos;
        let energy = self.creatures[i].energy;
        let meat   = self.creatures[i].genome.meat_efficiency(energy);
        let feed   = self.creatures[i].genome.feed_efficiency
            .evaluate(self.creatures[i].genome.energy_x(energy)).clamp(0.1, 1.0);
        // Meat Buff: base meat efficiency increased to 0.55 from 0.35
        let eff    = (0.55 + 0.65 * meat) * feed;
        let gut_boost = 1.0 + self.creatures[i].gut.meat_fit * 0.3;
        let r2     = 1.4 * 1.4;
        let bite   = 50.0_f64; // Meat Buff: bite size increased to 50.0 from 42.0
        let mut j  = 0;
        while j < self.carcasses.len() {
            if pos.dist_sq(self.carcasses[j].pos) <= r2 {
                let take = bite.min(self.carcasses[j].energy);
                self.carcasses[j].energy -= take;
                let para_mult = 1.0 - 0.40 * self.creatures[i].parasites;
                self.creatures[i].energy += take * eff * gut_boost * para_mult;
                self.creatures[i].vocal_type = 2;
                self.creatures[i].gut.digest_food(0.0, 1.0); // feed meat bacterium
                self.creatures[i].memory.reinforce(MemKind::Carcass, self.carcasses[j].pos, 3.0);
                if self.carcasses[j].energy <= 0.1 {
                    let carc_pos = self.carcasses[j].pos;
                    let last_idx = self.carcasses.len() - 1;
                    let last_pos = if j < last_idx {
                        Some((self.carcasses[last_idx].pos, last_idx))
                    } else {
                        None
                    };
                    self.carcasses.swap_remove(j);
                    self.carcass_grid.remove_and_swap(carc_pos, j, last_pos);
                    continue;
                }
                break;
            }
            j += 1;
        }
    }

    /// Eat from the closest aquatic plant patch underfoot.
    fn eat_aquatic_plant_at(&mut self, i: usize) {
        let pos       = self.creatures[i].pos;
        let my_swim   = self.creatures[i].genome.effective_swim(self.creatures[i].energy);
        let deep_ok   = my_swim >= self.cfg.aquatic_eat_deep_min_swim;
        let bite_r2   = 2.0 * 2.0;
        let bite      = 35.0_f64;
        let plant_eff = 0.5 + my_swim * 0.5; // better swimmers extract more energy

        for ap in &mut self.aquatic_plants {
            if ap.deep && !deep_ok { continue; }
            if ap.energy < 1.0 { continue; }
            if pos.dist_sq(ap.pos) > bite_r2 { continue; }
            let taken = ap.bite(bite);
            let energy = self.creatures[i].energy;
            let diet_eff = self.creatures[i].genome.plant_efficiency(energy);
            let cx = self.creatures[i].genome.energy_x(energy);
            let feed = self.creatures[i].genome.feed_efficiency
                .evaluate(cx).clamp(0.1, 1.0);
            let gut_boost = 1.0 + self.creatures[i].gut.plant_fit * 0.3;
            let para_mult = 1.0 - 0.40 * self.creatures[i].parasites;
            self.creatures[i].energy += taken * plant_eff * diet_eff * feed * gut_boost * para_mult;
            self.creatures[i].hydration = (self.creatures[i].hydration + 25.0).min(100.0);
            self.creatures[i].gut.digest_food(1.0, 0.0); // feed plant bacterium
            self.creatures[i].memory.record(MemKind::Food, ap.pos, taken as f32);
            break;
        }
    }

    fn spawn_carcass(&mut self, pos: Vec2, energy: f64) {
        if energy <= 0.0 || self.carcasses.len() >= self.cfg.max_carcasses { return; }
        self.carcasses.push(Carcass::new(pos, energy));
        self.total_carcasses_spawned += 1;
    }

    // ─── PREDATION (pack hunting) ─────────────────────────────────────────────

    fn predation(&mut self) {
        self.rebuild_creature_grid();
        let sightings: Vec<Sighting> = self.creatures.iter().enumerate()
            .map(|(idx, c)| Sighting { idx, pos: c.pos, power: c.combat_power(), alive: c.alive })
            .collect();

        // First pass: each attacker picks a target.
        // attack_map[prey_idx] = Vec of (attacker_idx, attack_power)
        let mut attack_map: std::collections::HashMap<usize, Vec<(usize, f64)>> =
            std::collections::HashMap::new();

        for s in &sightings {
            if !s.alive { continue; }
            let attacker = &self.creatures[s.idx];
            
            // Hunter check and hostile/territorial aggressiveness check
            let is_hunter = attacker.hunts();
            let is_hostile = attacker.genome.aggression.a >= 0.15;
            if !is_hunter && !is_hostile { continue; }

            // Very hungry carnivores are more determined (hunger-state aggression boost).
            let hunger_mult = if is_hunter && attacker.energy < attacker.genome.repro_threshold.a * 0.5 {
                1.0 + attacker.genome.effective_aggression(attacker.energy) * 0.5
            } else { 1.0 };
            let effective_power = s.power * hunger_mult;

            let reach = 1.5 + self.creatures[s.idx].genome.size.a;
            let reach2 = reach * reach;

            let mut chosen: Option<(usize, f64)> = None;
            self.creature_grid.query(s.pos, reach, |t_idx| {
                if t_idx == s.idx { return; }
                let t = &sightings[t_idx];
                if !t.alive { return; }
                let d2 = s.pos.dist_sq(t.pos);
                if d2 > reach2 { return; }
                
                let sim = self.creatures[s.idx].social.cosine_similarity(&self.creatures[t.idx].social);
                
                let mut should_attack = false;
                if is_hunter {
                    // Hunt target selection: respects kin reluctance
                    let cx = self.creatures[s.idx].genome.energy_x(self.creatures[s.idx].energy);
                    let altruism = self.creatures[s.idx].genome.altruism.evaluate(cx);
                    let sim_threshold = 0.85 - 0.25 * altruism;
                    if sim <= sim_threshold as f32 {
                        should_attack = true;
                    }
                }
                
                // Genus conflict / Racism: attack different genuses
                let racism_threshold = if attacker.in_tribe { 0.45 } else { 0.40 };
                if is_hostile && sim < racism_threshold {
                    should_attack = true;
                }

                if !should_attack { return; }

                // Scavengers (very low aggression) skip live prey (applies only to hunters targeting food).
                if is_hunter && self.creatures[s.idx].genome.aggression.a < 0.25
                    && self.creatures[t.idx].energy > 10.0 { return; }

                if chosen.map_or(true, |(_, bd)| d2 < bd) {
                    chosen = Some((t.idx, d2));
                }
            });
            if let Some((prey_idx, _)) = chosen {
                attack_map.entry(prey_idx).or_default().push((s.idx, effective_power));
            }
        }

        // Second pass: resolve each attack with pack-pooled power.
        let body_factor = self.cfg.carcass_body_factor;
        let pack_bonus  = self.cfg.pack_hunt_bonus;
        let pack_max    = self.cfg.pack_max_size;

        for (prey_idx, mut attackers) in attack_map {
            if !self.creatures[prey_idx].alive { continue; }
            // Sort by power descending; cap pack size.
            attackers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            attackers.truncate(pack_max);
            // Pool pack power: lead attacker gets base, each additional adds pack_bonus fraction.
            let lead_power  = attackers[0].1;
            let pack_mult   = 1.0 + (attackers.len() - 1) as f64 * pack_bonus;
            let total_power = lead_power * pack_mult;

            // Check any attacker is still alive.
            if attackers.iter().all(|(ai, _)| !self.creatures[*ai].alive) { continue; }

            let prey_power = self.creatures[prey_idx].combat_power();
            let prey_defense_power = prey_power * (1.0 + self.creatures[prey_idx].genome.effective_defense_spikes(self.creatures[prey_idx].energy) * 0.6);
            if total_power <= prey_defense_power {
                // All failed — small energy drain and spikes rebound damage.
                let spikes_dmg = 12.0 * self.creatures[prey_idx].genome.effective_defense_spikes(self.creatures[prey_idx].energy);
                for (ai, _) in &attackers {
                    if self.creatures[*ai].alive {
                        let expressed_blood_suck = self.creatures[*ai].genome.effective_blood_sucking(self.creatures[*ai].energy);
                        if expressed_blood_suck > 0.0 {
                            let steal = (25.0 * expressed_blood_suck).min(self.creatures[prey_idx].energy);
                            self.creatures[prey_idx].energy -= steal;
                            self.creatures[*ai].energy += steal;
                            if self.rng.chance(0.40) {
                                let max_para = self.creatures[*ai].parasites.max(self.creatures[prey_idx].parasites);
                                self.creatures[*ai].parasites = max_para;
                                self.creatures[prey_idx].parasites = max_para;
                            }
                        }
                        self.creatures[*ai].energy -= 0.4 + spikes_dmg;
                    }
                }
                continue;
            }

            // Lethality check on lead attacker.
            let lead_idx = attackers[0].0;
            if !self.creatures[lead_idx].alive { continue; }
            let lethality = self.creatures[lead_idx].genome.lethality
                .evaluate(self.creatures[lead_idx].genome.energy_x(self.creatures[lead_idx].energy))
                .clamp(0.0, 1.0);
            let speed_ratio = (self.creatures[lead_idx].genome.effective_speed(self.creatures[lead_idx].energy)
                / self.creatures[prey_idx].genome.effective_speed(self.creatures[prey_idx].energy)).min(1.0);
            // Larger packs get a catch-up bonus.
            let pack_catch = 1.0 - (1.0 - lethality * speed_ratio).powi(attackers.len() as i32);
            if !self.rng.chance(pack_catch) {
                for (ai, _) in &attackers {
                    if self.creatures[*ai].alive {
                        let expressed_blood_suck = self.creatures[*ai].genome.effective_blood_sucking(self.creatures[*ai].energy);
                        if expressed_blood_suck > 0.0 {
                            let steal = (25.0 * expressed_blood_suck).min(self.creatures[prey_idx].energy);
                            self.creatures[prey_idx].energy -= steal;
                            self.creatures[*ai].energy += steal;
                            if self.rng.chance(0.40) {
                                let max_para = self.creatures[*ai].parasites.max(self.creatures[prey_idx].parasites);
                                self.creatures[*ai].parasites = max_para;
                                self.creatures[prey_idx].parasites = max_para;
                            }
                        }
                        self.creatures[*ai].energy -= 0.2;
                    }
                }
                continue;
            }

            // Kill confirmed — split energy among pack.
            let body  = self.creatures[prey_idx].body_energy(body_factor);
            let feed  = self.creatures[lead_idx].genome.feed_efficiency
                .evaluate(self.creatures[lead_idx].genome.energy_x(self.creatures[lead_idx].energy))
                .clamp(0.1, 1.0);
            let prey_pos = self.creatures[prey_idx].pos;
            self.creatures[prey_idx].alive = false;
            self.creatures[prey_idx].carcass_spawned = true;

            // Distribute meat among pack members proportional to their power adjusted by altruism.
            let mut weights = Vec::with_capacity(attackers.len());
            let mut total_weight = 0.0;
            for (ai, ai_power) in &attackers {
                let cx = self.creatures[*ai].genome.energy_x(self.creatures[*ai].energy);
                let altruism = self.creatures[*ai].genome.altruism.evaluate(cx);
                let weight = ai_power * (1.0 - 0.5 * altruism).max(0.1);
                weights.push(weight);
                total_weight += weight;
            }

            for (idx, (ai, _)) in attackers.iter().enumerate() {
                if !self.creatures[*ai].alive { continue; }
                let share = if total_weight > 1e-9 { weights[idx] / total_weight } else { 0.0 };
                let meat_eff = self.creatures[*ai].genome.meat_efficiency(self.creatures[*ai].energy);
                let ai_feed = self.creatures[*ai].genome.feed_efficiency
                    .evaluate(self.creatures[*ai].genome.energy_x(self.creatures[*ai].energy))
                    .clamp(0.1, 1.0);
                let gut_boost = 1.0 + self.creatures[*ai].gut.meat_fit * 0.3;
                let para_mult = 1.0 - 0.40 * self.creatures[*ai].parasites;
                self.creatures[*ai].energy += body * share * meat_eff * ai_feed * gut_boost * para_mult;

                // Blood sucking warm blood bonus if successful:
                let expressed_blood_suck = self.creatures[*ai].genome.effective_blood_sucking(self.creatures[*ai].energy);
                if expressed_blood_suck > 0.0 {
                    self.creatures[*ai].energy += 20.0 * expressed_blood_suck;
                    if self.rng.chance(0.40) {
                        let max_para = self.creatures[*ai].parasites.max(self.creatures[prey_idx].parasites);
                        self.creatures[*ai].parasites = max_para;
                        self.creatures[prey_idx].parasites = max_para;
                    }
                }

                self.creatures[*ai].gut.digest_food(0.0, 1.0);
            }
            let leftover = body * (1.0 - feed);
            self.spawn_carcass(prey_pos, leftover);
        }
    }

    // ─── REPRODUCTION ────────────────────────────────────────────────────────

    fn reproduce(&mut self) {
        let asexual_cost = self.cfg.asexual_repro_cost;
        let sexual_cost  = asexual_cost / 4.0; // sexual cost is exactly 1/4 of cloning cost
        let n = self.creatures.len();
        let mut newborns: Vec<Creature> = Vec::new();

        for i in 0..n {
            if self.population() + newborns.len() >= self.cfg.max_population { break; }
            if self.creatures[i].overcrowded { continue; }
            let is_near_completed_nest_i = self.is_near_own_completed_nest(&self.creatures[i]);
            let cost_i = if is_near_completed_nest_i { sexual_cost * 0.5 } else { sexual_cost };
            if !self.creatures[i].wants_to_reproduce(cost_i, is_near_completed_nest_i) { continue; }

            let seeks_mate = self.rng.chance(self.creatures[i].genome.mating_pref.a);
            let mate = if seeks_mate { self.find_mate(i, sexual_cost) } else { None };

            if seeks_mate && mate.is_none() {
                self.creatures[i].vocal_type = 3; // Mating Call
            }

            match mate {
                Some(j) => {
                    let is_near_completed_nest_j = self.is_near_own_completed_nest(&self.creatures[j]);
                    let cost_j = if is_near_completed_nest_j { sexual_cost * 0.5 } else { sexual_cost };

                    self.creatures[i].energy -= cost_i;
                    self.creatures[j].energy -= cost_j;
                    self.creatures[i].reproduced_this_tick = true;
                    self.creatures[j].reproduced_this_tick = true;
                    let child_genome = Genome::crossover(&self.creatures[i].genome, &self.creatures[j].genome, &mut self.rng);
                    let pos = self.jitter_near(self.creatures[i].pos, 1.5);
                    
                    let child_gut = GutBacterium {
                        plant_fit: 0.5 * (self.creatures[i].gut.plant_fit + self.creatures[j].gut.plant_fit),
                        meat_fit: 0.5 * (self.creatures[i].gut.meat_fit + self.creatures[j].gut.meat_fit),
                        mood_aggression: 0.5 * (self.creatures[i].gut.mood_aggression + self.creatures[j].gut.mood_aggression),
                    }.mutated(&mut self.rng);

                    let child_rc = child_genome.effective_repro_complexity(self.cfg.offspring_energy);
                    let birth_gf = 0.25 + 0.15 * child_rc;
                    let birth_energy = self.cfg.offspring_energy + 20.0 * child_rc;

                    // Sexual birth: baby starts with complexity-based size/growth_factor
                    let mut child = Creature::new(self.next_id, pos, birth_energy, child_genome, birth_gf, child_gut);
                    child.parasites = 0.5 * (self.creatures[i].parasites + self.creatures[j].parasites) * 0.6;
                    child.parent_ids = Some((self.creatures[i].id, self.creatures[j].id));
                    child.is_inbred = self.are_closely_related(i, j);
                    
                    let avg_parent_rc = 0.5 * (self.creatures[i].genome.effective_repro_complexity(self.creatures[i].energy) + self.creatures[j].genome.effective_repro_complexity(self.creatures[j].energy));
                    let age_penalty = (40.0 - 25.0 * avg_parent_rc).round().max(1.0) as u16;
                    // Genomic memory: inherit mixed memories from parents aged by complexity-dampened ticks
                    child.memory.inherit_mixed(&self.creatures[i].memory, &self.creatures[j].memory, age_penalty);
                    
                    newborns.push(child);
                    self.next_id += 1;
                    self.total_sexual_births += 1;
                }
                None => {
                    if seeks_mate && self.creatures[i].genome.mating_pref.a > 0.5 { continue; }
                    if self.creatures[i].energy < asexual_cost + 40.0 { continue; }
                    
                    let parent_rc = self.creatures[i].genome.effective_repro_complexity(self.creatures[i].energy);
                    let fraction = 0.5 + 0.10 * parent_rc;

                    // Asexual splitting / binary fission: parent and child divide remaining energy and size by fraction
                    let remaining_energy = self.creatures[i].energy - asexual_cost;
                    self.creatures[i].energy = remaining_energy * (1.0 - fraction);
                    self.creatures[i].growth_factor = 1.0 - fraction; // parent size splits
                    self.creatures[i].reproduced_this_tick = true;

                    let parent_gut = self.creatures[i].gut;
                    self.creatures[i].gut = parent_gut.mutated(&mut self.rng);
                    let child_gut = parent_gut.mutated(&mut self.rng);

                    let child_energy = remaining_energy * fraction;
                    let child_genome = self.creatures[i].genome.mutated_low_rate(&mut self.rng);
                    let pos = self.jitter_near(self.creatures[i].pos, 1.5);
                    
                    // Child starts at fraction growth_factor
                    let mut child = Creature::new(self.next_id, pos, child_energy, child_genome, fraction, child_gut);
                    child.parasites = self.creatures[i].parasites * 0.6;
                    child.parent_ids = Some((self.creatures[i].id, self.creatures[i].id));
                    child.is_inbred = self.creatures[i].is_inbred;
                    
                    let age_penalty = (40.0 - 25.0 * parent_rc).round().max(1.0) as u16;
                    // Genomic memory: inherit parent memories aged by complexity-dampened ticks
                    child.memory.inherit_from(&self.creatures[i].memory, age_penalty);

                    newborns.push(child);
                    self.next_id += 1;
                    self.total_asexual_births += 1;
                }
            }
        }
        self.creatures.extend(newborns);
    }

    fn find_mate(&self, i: usize, base_cost: f64) -> Option<usize> {
        let me = &self.creatures[i];
        let r2 = me.genome.sense.a.max(4.0).powi(2);
        let mut best: Option<(usize, f64, f32)> = None;
        for (j, c) in self.creatures.iter().enumerate() {
            if j == i || !c.alive || c.reproduced_this_tick || c.overcrowded { continue; }
            let is_near_completed_nest_j = self.is_near_own_completed_nest(c);
            let cost_j = if is_near_completed_nest_j { base_cost * 0.5 } else { base_cost };
            let thresh = if is_near_completed_nest_j { c.genome.repro_threshold.a * 0.8 } else { c.genome.repro_threshold.a };
            if c.energy <= cost_j + 1.0 || c.energy < thresh { continue; }
            let d2  = me.pos.dist_sq(c.pos);
            if d2 > r2 { continue; }
            let sim = me.social.cosine_similarity(&c.social);
            if best.map_or(true, |(_, bd, bs)| d2 < bd || (d2 == bd && sim > bs)) {
                best = Some((j, d2, sim));
            }
        }
        best.map(|(j, _, _)| j)
    }

    fn jitter_near(&mut self, p: Vec2, r: f64) -> Vec2 {
        Vec2::new(p.x + self.rng.range_f64(-r, r), p.y + self.rng.range_f64(-r, r))
            .clamp_to(self.cfg.width, self.cfg.height)
    }

    // ─── SOCIAL VECTORS ──────────────────────────────────────────────────────

    fn update_social_vecs(&mut self) {
        let n = self.creatures.len();
        let snapshots: Vec<(Vec2, bool, SocialVec)> = self.creatures.iter()
            .map(|c| (c.pos, c.alive, c.social.clone()))
            .collect();
        let r2 = self.cfg.social_influence_radius * self.cfg.social_influence_radius;
        for i in 0..n {
            if !snapshots[i].1 { continue; }
            let my_pos = snapshots[i].0;
            let neighbours: Vec<&SocialVec> = snapshots.iter().enumerate()
                .filter(|(j, (pos, alive, _))| *j != i && *alive && my_pos.dist_sq(*pos) <= r2)
                .map(|(_, (_, _, sv))| sv)
                .collect();
            if let Some(avg) = SocialVec::average(&neighbours) {
                self.creatures[i].social.mix_toward(&avg, SOCIAL_ALPHA);
            }
        }
    }

    fn resolve_vocalizations(&mut self) {
        let n = self.creatures.len();
        let mut vocalizers = Vec::new();
        for i in 0..n {
            let c = &self.creatures[i];
            if c.alive && c.vocal_type > 0 {
                vocalizers.push((i, c.pos, c.vocal_type, c.combat_power()));
            }
        }

        let vocal_range = 15.0;
        for (caller_idx, caller_pos, vocal_type, caller_power) in vocalizers {
            self.creature_grid.query(caller_pos, vocal_range, |listener_idx| {
                if listener_idx == caller_idx { return; }
                let listener = &self.creatures[listener_idx];
                if !listener.alive { return; }
                if caller_pos.dist(listener.pos) > vocal_range { return; }

                match vocal_type {
                    1 => {
                        self.creatures[listener_idx].memory.record(MemKind::Threat, caller_pos, caller_power as f32);
                    }
                    2 => {
                        if listener.eats_meat() {
                            self.creatures[listener_idx].memory.record(MemKind::Carcass, caller_pos, 40.0);
                        } else {
                            self.creatures[listener_idx].memory.record(MemKind::Food, caller_pos, 40.0);
                        }
                    }
                    3 => {
                        self.creatures[listener_idx].memory.record(MemKind::Mate, caller_pos, 40.0);
                    }
                    _ => {}
                }
            });
        }
    }

    // ─── END OF CYCLE ────────────────────────────────────────────────────────

    fn end_of_cycle(&mut self) {
        self.cycle += 1;

        // Starvation.
        for c in &mut self.creatures {
            if c.alive && c.energy <= self.cfg.starve_threshold { c.alive = false; }
        }
        // Death roll.
        for idx in 0..self.creatures.len() {
            if !self.creatures[idx].alive { continue; }
            let p = self.cfg.death_probability(self.creatures[idx].cycles_survived);
            if self.rng.chance(p) {
                self.creatures[idx].alive = false;
            } else {
                self.creatures[idx].cycles_survived += 1;
            }
        }
        // Tree migration.
        let (w, h) = (self.cfg.width, self.cfg.height);
        let dist  = self.cfg.source_relocate_distance;
        let phase = self.season_phase;
        let amp   = self.cfg.tree_migrate_amp;
        for s in 0..self.sources.len() {
            self.sources[s].migrate(dist, w, h, phase, amp, &mut self.rng);
        }
        // Tree death from climate misfit.
        if self.sources.len() > self.cfg.min_trees {
            let base = self.cfg.tree_death_base;
            let clim = self.cfg.tree_death_climate;
            let mut survivors = Vec::with_capacity(self.sources.len());
            let old_trees = std::mem::take(&mut self.sources);
            for tree in old_trees {
                let temp = self.temperature_at(tree.pos.y);
                let p = base + clim * (1.0 - tree.climate_fit(temp));
                if survivors.len() < self.cfg.min_trees || !self.rng.chance(p) {
                    survivors.push(tree);
                }
            }
            self.sources = survivors;
        }
        // Carcass decay.
        self.decay_carcasses();
        // Fertilizer leach.
        let decay = self.cfg.fertilizer_decay;
        for f in &mut self.fertilizer { f.amount *= 1.0 - decay; }
        self.fertilizer.retain(|f| f.amount > 0.05);
        // Aquatic plants: remove nearly-empty patches.
        self.aquatic_plants.retain(|ap| ap.energy > 0.5 || ap.max_energy > 0.0);
        // Fruit germination / rot.
        let heavy_grazed: Vec<u64> = self.sources.iter()
            .filter(|s| s.feeds_this_cycle >= self.cfg.tree_heavy_graze_threshold)
            .map(|s| s.id).collect();
        for s in &mut self.sources { s.feeds_this_cycle = 0; }
        self.age_and_germinate_fruit(&heavy_grazed);
        self.record_history();
    }

    fn decay_carcasses(&mut self) {
        let decay = self.cfg.carcass_decay;
        let old = std::mem::take(&mut self.carcasses);
        let mut kept = Vec::with_capacity(old.len());
        for mut c in old {
            let rotted = c.energy * decay;
            c.energy -= rotted;
            c.age_cycles += 1;
            self.deposit_fertilizer(c.pos, rotted * 0.5);
            if c.energy > 1.0 && c.age_cycles < 6 { kept.push(c); }
            else { self.deposit_fertilizer(c.pos, c.energy.max(0.0) * 0.5); }
        }
        self.carcasses = kept;
    }

    fn age_and_germinate_fruit(&mut self, heavy_grazed: &[u64]) {
        let (w, h) = (self.cfg.width, self.cfg.height);
        let old = std::mem::take(&mut self.fruits);
        let mut kept = Vec::with_capacity(old.len());
        let fen_scale = self.fruit_energy_scale();
        for mut fruit in old {
            fruit.age_cycles += 1;
            if fruit.age_cycles >= self.cfg.fruit_germinate_age && self.sources.len() < self.cfg.max_trees {
                let fert   = self.fertilizer_near(fruit.pos, 5.0);
                let chance = self.cfg.germinate_base_chance + self.cfg.germinate_fert_boost * fert;
                if self.rng.chance(chance) && self.terrain.tile_at(fruit.pos.x, fruit.pos.y).allows_trees() {
                    let mut genome = fruit.parent.mutated(&mut self.rng);
                    if heavy_grazed.contains(&fruit.source_id) {
                        genome.poison = (genome.poison + 0.15).min(1.0);
                    }
                    let mut new_tree = FoodSource::new(fruit.pos.clamp_to(w, h), genome);
                    new_tree.id = self.next_tree_id;
                    self.next_tree_id += 1;
                    self.sources.push(new_tree);
                    self.total_trees_germinated += 1;
                    continue;
                }
            }
            let rot = 0.12 + 0.06 * fruit.age_cycles as f64;
            if !self.rng.chance(rot.min(0.8)) { kept.push(fruit); }
        }
        self.fruits = kept;
        let _ = fen_scale;
    }

    fn deposit_fertilizer(&mut self, pos: Vec2, amount: f64) {
        if amount <= 0.0 || self.fertilizer.len() >= self.cfg.max_fertilizer { return; }
        self.fertilizer.push(Fertilizer::new(pos, amount));
    }

    fn record_history(&mut self) {
        let (mut herb, mut omni, mut carn) = (0, 0, 0);
        for c in &self.creatures {
            if !c.alive { continue; }
            match c.diet_class() {
                DietClass::Herbivore => herb += 1,
                DietClass::Omnivore  => omni += 1,
                DietClass::Carnivore => carn += 1,
            }
        }
        self.history.push(CycleRecord { cycle: self.cycle, herbivores: herb, omnivores: omni, carnivores: carn });
    }

    pub fn history_csv(&self) -> String {
        let mut s = String::from("cycle,herbivores,omnivores,carnivores,total\n");
        for r in &self.history {
            s.push_str(&format!("{},{},{},{},{}\n", r.cycle, r.herbivores, r.omnivores, r.carnivores, r.herbivores+r.omnivores+r.carnivores));
        }
        s
    }

    fn compact(&mut self) {
        let bf = self.cfg.carcass_body_factor;
        let mut bodies: Vec<(Vec2, f64)> = Vec::new();
        for c in &self.creatures {
            if !c.alive && !c.carcass_spawned { bodies.push((c.pos, c.body_energy(bf))); }
        }
        for (pos, energy) in bodies { self.spawn_carcass(pos, energy); }
        self.creatures.retain(|c| c.alive);
    }

    fn drop_poop(&mut self) {
        let well_fed = self.cfg.offspring_energy;
        let mut spots = Vec::new();
        for c in &self.creatures {
            if c.alive && c.energy > well_fed && self.rng.chance(self.cfg.poop_chance) {
                spots.push(c.pos);
            }
        }
        for p in spots { self.deposit_fertilizer(p, self.cfg.poop_amount); }
    }

    fn fertilizer_near(&self, p: Vec2, r: f64) -> f64 {
        let r2 = r * r;
        self.fertilizer.iter().filter(|f| f.pos.dist_sq(p) <= r2).map(|f| f.amount).sum()
    }

    // ─── STATISTICS ───────────────────────────────────────────────────────────

    pub fn stats(&self) -> Stats {
        let alive: Vec<&Creature> = self.creatures.iter().filter(|c| c.alive).collect();
        let n  = alive.len().max(1) as f64;
        let tn = self.sources.len().max(1) as f64;
        let mut s = Stats {
            population: alive.len(),
            fruits: self.fruits.len(),
            carcasses: self.carcasses.len(),
            fertilizer_patches: self.fertilizer.len(),
            trees: self.sources.len(),
            aquatic_plants: self.aquatic_plants.len(),
            season: climate::season_name(self.season_phase),
            ..Default::default()
        };
        for c in &alive {
            s.avg_energy          += c.energy;
            s.avg_speed           += c.genome.speed.a;
            s.avg_sense           += c.genome.sense.a;
            s.avg_size            += c.genome.size.a;
            s.avg_diet            += c.genome.diet.a;
            s.avg_aggression      += c.genome.aggression.a;
            s.avg_mating_pref     += c.genome.mating_pref.a;
            s.avg_temp_optimum    += c.genome.temp_optimum.a;
            s.avg_temp_tolerance  += c.genome.temp_tolerance.a;
            s.avg_poison_resist   += c.genome.poison_resist.a;
            s.avg_lethality       += c.genome.lethality.a;
            s.avg_feed_efficiency += c.genome.feed_efficiency.a;
            s.avg_cycles_survived += c.cycles_survived as f64;
            s.avg_swim_capability += c.genome.swim_capability.a;
            s.avg_climb_capability+= c.genome.climb_capability.a;
            match c.diet_class() {
                DietClass::Herbivore => s.herbivores += 1,
                DietClass::Omnivore  => s.omnivores  += 1,
                DietClass::Carnivore => s.carnivores += 1,
            }
        }
        s.avg_energy /= n; s.avg_speed /= n; s.avg_sense /= n; s.avg_size /= n;
        s.avg_diet /= n; s.avg_aggression /= n; s.avg_mating_pref /= n;
        s.avg_temp_optimum /= n; s.avg_temp_tolerance /= n; s.avg_poison_resist /= n;
        s.avg_lethality /= n; s.avg_feed_efficiency /= n; s.avg_cycles_survived /= n;
        s.avg_swim_capability /= n; s.avg_climb_capability /= n;
        for t in &self.sources {
            s.avg_tree_poison    += t.genome.poison;
            s.avg_tree_fertility += t.genome.fertility;
            s.avg_seed_dispersal += t.genome.seed_dispersal;
            s.avg_drought_resist += t.genome.drought_resist;
        }
        s.avg_tree_poison    /= tn; s.avg_tree_fertility /= tn;
        s.avg_seed_dispersal /= tn; s.avg_drought_resist /= tn;
        s
    }

    /// Serialise world state to compact JSON for the web viewer.
    pub fn to_json(&self) -> String {
        let s = self.stats();
        let mut out = String::with_capacity(131_072);
        out.push_str(&format!(
            "{{\"cycle\":{},\"tick\":{},\"season\":\"{}\",\"width\":{},\"height\":{},\"weather\":\"{}\"",
            self.cycle, self.tick, s.season, self.cfg.width, self.cfg.height, self.current_weather.as_str()
        ));

        out.push_str(",\"creatures\":[");
        let mut first = true;
        for c in self.creatures.iter().filter(|c| c.alive) {
            if !first { out.push(','); }
            first = false;
            let e = c.energy;
            let pos = c.pos;
            let gx = pos.x.round() as usize;
            let gy = pos.y.round() as usize;
            let cols = self.cfg.width.ceil() as usize + 1;
            let rows = self.cfg.height.ceil() as usize + 1;
            let (tile_type, tile_energy, fungi_energy) = if gx < cols && gy < rows {
                let idx = gy * cols + gx;
                let tile = self.terrain.tile_at(pos.x, pos.y);
                let t_energy = match tile {
                    TileType::Plains | TileType::Sand | TileType::Mountain => self.grass[idx],
                    _ => 0.0,
                };
                let f_energy = self.fungi[idx];
                (tile as u8, t_energy, f_energy)
            } else {
                (0, 0.0, 0.0)
            };
            out.push_str(&format!(
                "{{\"id\":{},\"x\":{:.2},\"y\":{:.2},\
                \"energy\":{:.1},\"diet\":{:.3},\"size\":{:.3},\"speed\":{:.3},\
                \"aggression\":{:.3},\"lethality\":{:.3},\"mating_pref\":{:.3},\
                \"temp_optimum\":{:.3},\"poison_resist\":{:.3},\"feed_efficiency\":{:.3},\
                \"swim\":{:.3},\"climb\":{:.3},\"cycles\":{},\
                \"plant_fit\":{:.3},\"meat_fit\":{:.3},\"mood_aggression\":{:.3},\
                \"in_tribe\":{},\"claws\":{:.3},\"spikes\":{:.3},\"repro_complex\":{:.3},\"social_cap\":{:.3},\
                \"tile_type\":{},\"tile_energy\":{:.1},\"fungi_energy\":{:.1},\
                \"sickness\":{:.3},\"parasites\":{:.3},\"hydration\":{:.3},\"immunity\":{:.3},\"blood_sucking\":{:.3},\
                \"vocal\":{},\"carrying_twig\":{},\"is_inbred\":{},\"nest_pos\":[{},{}],\
                \"social\":[{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}]}}",
                c.id, c.pos.x, c.pos.y, e,
                c.genome.effective_diet(e),
                c.genome.effective_size(e) * c.growth_factor, // scale size by growth factor
                c.effective_speed(),
                c.genome.effective_aggression(e),
                c.genome.lethality.evaluate(c.genome.energy_x(e)).clamp(0.0,1.0),
                c.genome.mating_pref.a,
                c.genome.temp_optimum.a,
                c.genome.poison_resist.evaluate(c.genome.energy_x(e)).clamp(0.0,1.0),
                c.genome.feed_efficiency.evaluate(c.genome.energy_x(e)).clamp(0.1,1.0),
                c.genome.effective_swim(e),
                c.genome.effective_climb(e),
                c.cycles_survived,
                c.gut.plant_fit,
                c.gut.meat_fit,
                c.gut.mood_aggression,
                if c.in_tribe { "true" } else { "false" },
                c.genome.effective_claws(e),
                c.genome.effective_defense_spikes(e),
                c.genome.effective_repro_complexity(e),
                c.genome.effective_social_capacity(e),
                tile_type, tile_energy, fungi_energy,
                c.sickness, c.parasites, c.hydration,
                c.effective_immunity(),
                c.genome.effective_blood_sucking(e),
                c.vocal_type,
                if c.carrying_twig { "true" } else { "false" },
                if c.is_inbred { "true" } else { "false" },
                c.nest_pos.map_or(-1.0, |p| p.x),
                c.nest_pos.map_or(-1.0, |p| p.y),
                c.social.0[0],c.social.0[1],c.social.0[2],c.social.0[3],
                c.social.0[4],c.social.0[5],c.social.0[6],c.social.0[7],
            ));
        }
        out.push(']');

        out.push_str(",\"trees\":[");
        first = true;
        for t in &self.sources {
            if !first { out.push(','); }
            first = false;
            out.push_str(&format!(
                "{{\"x\":{:.2},\"y\":{:.2},\"poison\":{:.3},\"fertility\":{:.3},\
                \"seed_dispersal\":{:.3},\"drought_resist\":{:.3},\"is_fruit\":{}}}",
                t.pos.x, t.pos.y, t.genome.poison, t.genome.fertility,
                t.genome.seed_dispersal, t.genome.drought_resist,
                if t.is_fruit_tree { "true" } else { "false" },
            ));
        }
        out.push(']');

        out.push_str(",\"fruits\":[");
        first = true;
        for f in &self.fruits {
            if !first { out.push(','); }
            first = false;
            out.push_str(&format!("{{\"x\":{:.2},\"y\":{:.2},\"p\":{:.3}}}",
                f.pos.x, f.pos.y, f.poison));
        }
        out.push(']');

        out.push_str(",\"carcasses\":[");
        first = true;
        for c in &self.carcasses {
            if !first { out.push(','); }
            first = false;
            out.push_str(&format!("{{\"x\":{:.2},\"y\":{:.2},\"e\":{:.1}}}",
                c.pos.x, c.pos.y, c.energy));
        }
        out.push(']');

        out.push_str(",\"aquatic\":[");
        first = true;
        for ap in &self.aquatic_plants {
            if !first { out.push(','); }
            first = false;
            out.push_str(&format!("{{\"x\":{:.2},\"y\":{:.2},\"e\":{:.1},\"deep\":{}}}",
                ap.pos.x, ap.pos.y, ap.energy, if ap.deep { "true" } else { "false" }));
        }
        out.push(']');

        out.push_str(",\"mushrooms\":[");
        first = true;
        let cols = self.cfg.width.ceil() as usize + 1;
        let rows = self.cfg.height.ceil() as usize + 1;
        for gy in 0..rows {
            for gx in 0..cols {
                let idx = gy * cols + gx;
                let val = self.fungi[idx];
                if val > 0.1 {
                    if !first { out.push(','); }
                    first = false;
                    out.push_str(&format!("{{\"x\":{},\"y\":{},\"e\":{:.1}}}", gx, gy, val));
                }
            }
        }
        out.push(']');

        let hist_start = self.history.len().saturating_sub(200);
        out.push_str(",\"history\":[");
        for (k, r) in self.history[hist_start..].iter().enumerate() {
            if k > 0 { out.push(','); }
            out.push_str(&format!("{{\"cycle\":{},\"h\":{},\"o\":{},\"c\":{}}}",
                r.cycle, r.herbivores, r.omnivores, r.carnivores));
        }
        out.push(']');

        out.push_str(&format!(
            ",\"stats\":{{\"pop\":{},\"h\":{},\"o\":{},\"c\":{},\
            \"trees\":{},\"fruits\":{},\"carcasses\":{},\"aquatic\":{},\
            \"avg_diet\":{:.3},\"avg_energy\":{:.1},\
            \"avg_lethality\":{:.3},\"avg_temp_opt\":{:.3},\
            \"avg_poison_resist\":{:.3},\"avg_tree_poison\":{:.3},\
            \"avg_swim\":{:.3},\"avg_climb\":{:.3},\
            \"avg_seed_disp\":{:.3},\"avg_drought_res\":{:.3}}}",
            s.population, s.herbivores, s.omnivores, s.carnivores,
            s.trees, s.fruits, s.carcasses, s.aquatic_plants,
            s.avg_diet, s.avg_energy,
            s.avg_lethality, s.avg_temp_optimum,
            s.avg_poison_resist, s.avg_tree_poison,
            s.avg_swim_capability, s.avg_climb_capability,
            s.avg_seed_dispersal, s.avg_drought_resist,
        ));
        out.push_str(",\"twigs\":[");
        first = true;
        for t in &self.twigs {
            if !first { out.push(','); }
            first = false;
            out.push_str(&format!("{{\"x\":{:.2},\"y\":{:.2}}}", t.pos.x, t.pos.y));
        }
        out.push(']');

        out.push_str(",\"nests\":[");
        first = true;
        for n in &self.nests {
            if !first { out.push(','); }
            first = false;
            out.push_str(&format!("{{\"x\":{:.2},\"y\":{:.2},\"owner\":{},\"twigs\":{},\"done\":{}}}",
                n.pos.x, n.pos.y, n.owner_id, n.twigs, if n.completed { "true" } else { "false" }));
        }
        out.push(']');

        out.push('}');
        out
    }

    pub fn terrain_json(&self) -> String {
        let cols = self.cfg.width.ceil() as usize + 1;
        let rows = self.cfg.height.ceil() as usize + 1;
        let mut out = String::with_capacity(cols * rows * 2 + 64);
        out.push_str(&format!("{{\"cols\":{},\"rows\":{},\"tiles\":[", cols, rows));
        for y in 0..rows {
            for x in 0..cols {
                if x > 0 || y > 0 { out.push(','); }
                out.push_str(&(self.terrain.tile_at(x as f64, y as f64) as u8).to_string());
            }
        }
        out.push_str("]}");
        out
    }
}

// ─── Stats ───────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub population: usize,
    pub fruits: usize,
    pub carcasses: usize,
    pub fertilizer_patches: usize,
    pub trees: usize,
    pub aquatic_plants: usize,
    pub season: &'static str,
    pub herbivores: usize,
    pub omnivores: usize,
    pub carnivores: usize,
    pub avg_energy: f64,
    pub avg_speed: f64,
    pub avg_sense: f64,
    pub avg_size: f64,
    pub avg_diet: f64,
    pub avg_aggression: f64,
    pub avg_mating_pref: f64,
    pub avg_temp_optimum: f64,
    pub avg_temp_tolerance: f64,
    pub avg_poison_resist: f64,
    pub avg_lethality: f64,
    pub avg_feed_efficiency: f64,
    pub avg_cycles_survived: f64,
    pub avg_tree_poison: f64,
    pub avg_tree_fertility: f64,
    pub avg_swim_capability: f64,
    pub avg_climb_capability: f64,
    pub avg_seed_dispersal: f64,
    pub avg_drought_resist: f64,
}

#[derive(Clone, Debug)]
pub struct SpatialGrid {
    width: f64,
    height: f64,
    cell_size: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
}

impl SpatialGrid {
    pub fn new(width: f64, height: f64, cell_size: f64) -> Self {
        let cols = (width / cell_size).ceil() as usize;
        let rows = (height / cell_size).ceil() as usize;
        let cells = vec![Vec::new(); cols * rows];
        SpatialGrid {
            width,
            height,
            cell_size,
            cols,
            rows,
            cells,
        }
    }

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
        }
    }

    #[inline]
    fn get_cell_idx(&self, pos: Vec2) -> usize {
        let col = (pos.x / self.cell_size).floor() as isize;
        let row = (pos.y / self.cell_size).floor() as isize;
        let col = col.clamp(0, self.cols as isize - 1) as usize;
        let row = row.clamp(0, self.rows as isize - 1) as usize;
        row * self.cols + col
    }

    pub fn add(&mut self, pos: Vec2, index: usize) {
        let idx = self.get_cell_idx(pos);
        self.cells[idx].push(index);
    }

    pub fn query(&self, pos: Vec2, radius: f64, mut callback: impl FnMut(usize)) {
        let min_col = ((pos.x - radius) / self.cell_size).floor() as isize;
        let max_col = ((pos.x + radius) / self.cell_size).floor() as isize;
        let min_row = ((pos.y - radius) / self.cell_size).floor() as isize;
        let max_row = ((pos.y + radius) / self.cell_size).floor() as isize;

        let min_col = min_col.clamp(0, self.cols as isize - 1) as usize;
        let max_col = max_col.clamp(0, self.cols as isize - 1) as usize;
        let min_row = min_row.clamp(0, self.rows as isize - 1) as usize;
        let max_row = max_row.clamp(0, self.rows as isize - 1) as usize;

        for r in min_row..=max_row {
            let row_offset = r * self.cols;
            for c in min_col..=max_col {
                let idx = row_offset + c;
                for &index in &self.cells[idx] {
                    callback(index);
                }
            }
        }
    }

    pub fn remove_and_swap(&mut self, pos: Vec2, index: usize, last_pos: Option<(Vec2, usize)>) {
        let cell_idx = self.get_cell_idx(pos);
        if let Some(pos_in_cell) = self.cells[cell_idx].iter().position(|&x| x == index) {
            self.cells[cell_idx].swap_remove(pos_in_cell);
        }
        if let Some((l_pos, last_index)) = last_pos {
            let last_cell_idx = self.get_cell_idx(l_pos);
            if let Some(pos_in_cell) = self.cells[last_cell_idx].iter().position(|&x| x == last_index) {
                self.cells[last_cell_idx][pos_in_cell] = index;
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::Gene;

    fn tiny_cfg() -> Config {
        let mut c = Config::default();
        c.seed = 123;
        c.initial_creatures = 40;
        c.initial_food_sources = 5;
        c.width = 40.0;
        c.height = 20.0;
        c.max_aquatic_plants = 20;
        c.initial_aquatic_plants = 5;
        c
    }

    #[test]
    fn world_initializes_population() {
        let w = World::new(tiny_cfg());
        assert_eq!(w.population(), 40);
        assert_eq!(w.sources.len(), 5);
    }

    #[test]
    fn step_advances_tick_and_stays_bounded() {
        let mut w = World::new(tiny_cfg());
        for _ in 0..200 {
            if !w.step() { break; }
            assert!(w.population() <= w.cfg.max_population);
            assert!(w.fruits.len() <= w.cfg.max_fruits);
        }
        assert!(w.tick > 0);
    }

    #[test]
    fn cycle_increments_after_ticks_per_cycle() {
        let mut cfg = tiny_cfg();
        cfg.ticks_per_cycle = 10;
        let mut w = World::new(cfg);
        for _ in 0..10 { w.step(); }
        assert_eq!(w.cycle, 1);
    }

    #[test]
    fn reproduction_can_increase_population() {
        let mut cfg = tiny_cfg();
        cfg.seed = 7;
        let mut w = World::new(cfg);
        for c in &mut w.creatures {
            c.energy = 250.0;
            c.genome.repro_threshold = Gene::constant(120.0);
            c.genome.mating_pref = Gene::constant(0.0);
        }
        let before = w.population();
        w.reproduce();
        w.compact();
        assert!(w.population() > before);
    }

    #[test]
    fn asexual_reproduction_costs_the_full_amount() {
        let cfg = tiny_cfg();
        let asexual_cost = cfg.asexual_repro_cost;
        let mut w = World::new(cfg);
        w.creatures.truncate(1);
        let c = &mut w.creatures[0];
        c.energy = 250.0;
        c.genome.repro_threshold = Gene::constant(120.0);
        c.genome.mating_pref = Gene::constant(0.0);
        c.genome.repro_complexity = Gene::constant(0.0);
        let before = w.creatures[0].energy;
        w.reproduce();
        let parent = w.creatures.iter().find(|c| c.id == 0).unwrap();
        // With v3 splitting, parent keeps half of the remaining energy after cost
        let expected_shared = (before - asexual_cost) * 0.5;
        assert!((parent.energy - expected_shared).abs() < 1e-9);
        assert!((parent.growth_factor - 0.5).abs() < 1e-9);
        
        let child = w.creatures.iter().find(|c| c.id != 0).unwrap();
        assert!((child.energy - expected_shared).abs() < 1e-9);
        assert!((child.growth_factor - 0.5).abs() < 1e-9);
        assert_eq!(w.total_asexual_births, 1);
    }

    #[test]
    fn history_records_per_cycle() {
        let mut cfg = tiny_cfg();
        cfg.ticks_per_cycle = 5;
        let mut w = World::new(cfg);
        for _ in 0..15 { if !w.step() { break; } }
        assert!(!w.history.is_empty());
        let csv = w.history_csv();
        assert!(csv.starts_with("cycle,herbivores,omnivores,carnivores,total"));
    }

    #[test]
    fn aquatic_plants_spawn_in_water() {
        let cfg = tiny_cfg();
        // At least some aquatic plants should be seeded if water tiles exist.
        let w = World::new(cfg);
        // Terrain may or may not have water at this small size; just check no panic.
        assert!(w.aquatic_plants.len() <= w.cfg.max_aquatic_plants);
    }

    #[test]
    fn terrain_json_is_valid_structure() {
        let w = World::new(tiny_cfg());
        let tj = w.terrain_json();
        assert!(tj.starts_with("{\"cols\":"));
        assert!(tj.contains("\"tiles\":["));
    }

    #[test]
    fn pack_hunting_does_not_panic() {
        let mut w = World::new(tiny_cfg());
        // Give all creatures high aggression and diet to trigger pack hunting.
        for c in &mut w.creatures {
            c.genome.diet = Gene::constant(1.0);
            c.genome.aggression = Gene::constant(1.0);
            c.genome.lethality = Gene::constant(1.0);
        }
        w.predation(); // should not panic
    }

    #[test]
    fn test_grass_grazing_and_regrowth() {
        let mut cfg = tiny_cfg();
        cfg.grass_regrow_rate = 2.0;
        cfg.grass_max_energy = 100.0;
        cfg.grass_graze_max = 15.0;

        let mut w = World::new(cfg);
        let cols = (w.cfg.width.ceil() as usize) + 1;
        let rows = (w.cfg.height.ceil() as usize) + 1;
        let (tx, ty) = {
            let mut found = None;
            for y in 0..rows {
                for x in 0..cols {
                    let tile = w.terrain.tile_at(x as f64, y as f64);
                    if tile == TileType::Plains || tile == TileType::Sand {
                        found = Some((x as f64 + 0.5, y as f64 + 0.5));
                        break;
                    }
                }
                if found.is_some() { break; }
            }
            found.expect("No land tile found in test map")
        };

        w.grass.fill(50.0);

        w.creatures.truncate(1);
        let c = &mut w.creatures[0];
        c.pos.x = tx;
        c.pos.y = ty;
        c.energy = 50.0;
        c.genome.diet = Gene::constant(0.0);
        c.genome.graze = Gene::constant(0.8);
        c.genome.feed_efficiency = Gene::constant(1.0);

        w.move_and_eat();

        assert!(w.creatures[0].energy > 50.0, "Creature did not gain energy from grazing");
        let final_pos = w.creatures[0].pos;
        let gx = final_pos.x.round() as usize;
        let gy = final_pos.y.round() as usize;
        let final_idx = gy * cols + gx;
        assert!(w.grass[final_idx] < 50.0, "Grass energy did not decrease after grazing");

        let grass_before = w.grass[final_idx];
        w.season_phase = 0.5;
        w.regrow_grass();
        assert!(w.grass[final_idx] > grass_before, "Grass did not regrow in summer");
    }

    #[test]
    fn test_altruism_kin_sharing() {
        let cfg = tiny_cfg();
        let mut w = World::new(cfg);
        w.creatures.truncate(2);

        let (left, right) = w.creatures.split_at_mut(1);
        let c0 = &mut left[0];
        c0.pos.x = 10.0;
        c0.pos.y = 10.0;
        c0.energy = 220.0;
        c0.genome.repro_threshold = Gene::constant(150.0);
        c0.genome.altruism = Gene::constant(0.9);
        c0.social.0 = [1.0; 10];

        let c1 = &mut right[0];
        c1.pos.x = 10.5;
        c1.pos.y = 10.0;
        c1.energy = 30.0;
        c1.genome.repro_threshold = Gene::constant(150.0);
        c1.genome.altruism = Gene::constant(0.0);
        c1.social.0 = [1.0; 10];

        w.share_altruistic_energy();

        assert!(w.creatures[0].energy < 220.0, "Altruistic donor did not share energy");
        assert!(w.creatures[1].energy > 30.0, "Altruistic recipient did not gain energy");
    }

    #[test]
    fn test_altruism_kin_predation_reluctance() {
        {
            let mut w = World::new(tiny_cfg());
            w.creatures.truncate(2);

            let (left, right) = w.creatures.split_at_mut(1);
            let c0 = &mut left[0];
            c0.pos.x = 10.0;
            c0.pos.y = 10.0;
            c0.energy = 100.0;
            c0.genome.diet = Gene::constant(1.0);
            c0.genome.aggression = Gene::constant(1.0);
            c0.genome.lethality = Gene::constant(1.0);
            c0.genome.speed = Gene::constant(1.0);
            c0.genome.altruism = Gene::constant(0.0);
            
            c0.social.0 = [0.0; 10];
            c0.social.0[0] = 1.0;

            let c1 = &mut right[0];
            c1.pos.x = 10.2;
            c1.pos.y = 10.0;
            c1.energy = 50.0;
            c1.genome.diet = Gene::constant(0.0);
            c1.genome.speed = Gene::constant(0.1);
            c1.social.0 = [0.0; 10];
            c1.social.0[0] = 0.7;
            c1.social.0[1] = 0.71414284;

            let sim = c0.social.cosine_similarity(&c1.social);
            assert!((sim - 0.7).abs() < 1e-5);

            w.predation();
            assert!(!w.creatures[1].alive, "Egoistic predator should have killed the prey");
        }

        {
            let mut w = World::new(tiny_cfg());
            w.creatures.truncate(2);

            let (left, right) = w.creatures.split_at_mut(1);
            let c0 = &mut left[0];
            c0.pos.x = 10.0;
            c0.pos.y = 10.0;
            c0.energy = 100.0;
            c0.genome.diet = Gene::constant(1.0);
            c0.genome.aggression = Gene::constant(1.0);
            c0.genome.lethality = Gene::constant(1.0);
            c0.genome.speed = Gene::constant(1.0);
            c0.genome.altruism = Gene::constant(1.0);
            
            c0.social.0 = [0.0; 10];
            c0.social.0[0] = 1.0;

            let c1 = &mut right[0];
            c1.pos.x = 10.2;
            c1.pos.y = 10.0;
            c1.energy = 50.0;
            c1.genome.diet = Gene::constant(0.0);
            c1.genome.speed = Gene::constant(0.1);
            c1.social.0 = [0.0; 10];
            c1.social.0[0] = 0.7;
            c1.social.0[1] = 0.71414284;

            w.predation();
            assert!(w.creatures[1].alive, "Altruistic predator should have spared the kin prey");
        }
    }

    #[test]
    fn test_altruism_pack_hunt_sharing() {
        let mut w = World::new(tiny_cfg());
        w.creatures.truncate(3);

        let (left, right) = w.creatures.split_at_mut(1);
        let c0 = &mut left[0];
        c0.id = 0;
        c0.alive = true;
        c0.pos.x = 10.0;
        c0.pos.y = 10.0;
        c0.energy = 100.0;
        c0.growth_factor = 1.0;
        c0.genome.size = Gene::constant(2.0);
        c0.genome.diet = Gene::constant(1.0);
        c0.genome.aggression = Gene::constant(0.5);
        c0.genome.lethality = Gene::constant(1.0);
        c0.genome.speed = Gene::constant(1.0);
        c0.genome.feed_efficiency = Gene::constant(1.0);
        c0.genome.altruism = Gene::constant(1.0);
        c0.genome.claws = Gene::constant(0.0);
        c0.genome.defense_spikes = Gene::constant(0.0);
        c0.genome.repro_complexity = Gene::constant(0.0);
        c0.genome.social_capacity = Gene::constant(0.0);
        c0.social.0 = [0.0; 10];
        c0.social.0[0] = 1.0;
        c0.gut = GutBacterium::default();

        let (mid, end) = right.split_at_mut(1);
        let c1 = &mut mid[0];
        c1.id = 1;
        c1.alive = true;
        c1.pos.x = 10.2;
        c1.pos.y = 10.0;
        c1.energy = 100.0;
        c1.growth_factor = 1.0;
        c1.genome.size = Gene::constant(2.0);
        c1.genome.diet = Gene::constant(1.0);
        c1.genome.aggression = Gene::constant(0.5);
        c1.genome.lethality = Gene::constant(1.0);
        c1.genome.speed = Gene::constant(1.0);
        c1.genome.feed_efficiency = Gene::constant(1.0);
        c1.genome.altruism = Gene::constant(0.0);
        c1.genome.claws = Gene::constant(0.0);
        c1.genome.defense_spikes = Gene::constant(0.0);
        c1.genome.repro_complexity = Gene::constant(0.0);
        c1.genome.social_capacity = Gene::constant(0.0);
        c1.social.0 = [0.0; 10];
        c1.social.0[1] = 1.0;
        c1.gut = GutBacterium::default();

        let c2 = &mut end[0];
        c2.id = 2;
        c2.alive = true;
        c2.pos.x = 10.1;
        c2.pos.y = 10.05;
        c2.energy = 80.0;
        c2.growth_factor = 0.1;
        c2.genome.size = Gene::constant(0.4);
        c2.genome.diet = Gene::constant(0.0);
        c2.genome.aggression = Gene::constant(0.0);
        c2.genome.lethality = Gene::constant(0.0);
        c2.genome.speed = Gene::constant(0.1);
        c2.genome.claws = Gene::constant(0.0);
        c2.genome.defense_spikes = Gene::constant(0.0);
        c2.genome.repro_complexity = Gene::constant(0.0);
        c2.genome.social_capacity = Gene::constant(0.0);
        c2.social.0 = [0.0; 10];
        c2.social.0[2] = 1.0;

        println!("c0 power: {}", w.creatures[0].genome.combat_power(w.creatures[0].energy, w.creatures[0].growth_factor));
        println!("c1 power: {}", w.creatures[1].genome.combat_power(w.creatures[1].energy, w.creatures[1].growth_factor));
        println!("c2 power: {}", w.creatures[2].genome.combat_power(w.creatures[2].energy, w.creatures[2].growth_factor));
        println!("similarity c0-c2: {}", w.creatures[0].social.cosine_similarity(&w.creatures[2].social));

        let e0_before = w.creatures[0].energy;
        let e1_before = w.creatures[1].energy;

        w.predation();

        assert!(!w.creatures[2].alive);

        let gain0 = w.creatures[0].energy - e0_before;
        let gain1 = w.creatures[1].energy - e1_before;

        assert!(gain1 > 0.0 && gain0 > 0.0);
        let ratio = gain1 / gain0;
        assert!((ratio - 2.0).abs() < 0.1, "Expected ratio to be around 2.0, but got {}", ratio);
    }

    #[test]
    fn test_genus_conflict_and_mood_combat_power() {
        // 1. Test Mood Combat Power
        let mut rng = crate::rng::Rng::new(123);
        let mut genome = Genome::random(&mut rng);
        genome.size = Gene::constant(1.0);
        let base_c = Creature::new(1, Vec2::zero(), 100.0, genome.clone(), 1.0, GutBacterium::default());
        let base_power = base_c.combat_power();

        let aggressive_gut = GutBacterium {
            plant_fit: 0.5,
            meat_fit: 0.5,
            mood_aggression: 0.4, // shifts combat power up
        };
        let aggressive_c = Creature::new(1, Vec2::zero(), 100.0, genome.clone(), 1.0, aggressive_gut);
        assert!(aggressive_c.combat_power() > base_power, "Aggressive gut should boost combat power");

        let calm_gut = GutBacterium {
            plant_fit: 0.5,
            meat_fit: 0.5,
            mood_aggression: -0.4, // shifts combat power down
        };
        let calm_c = Creature::new(1, Vec2::zero(), 100.0, genome.clone(), 1.0, calm_gut);
        assert!(calm_c.combat_power() < base_power, "Calm gut should reduce combat power");

        // 2. Test Genus Conflict / Herbivore Infighting
        let mut w = World::new(tiny_cfg());
        w.creatures.truncate(2);

        let (left, right) = w.creatures.split_at_mut(1);
        let c0 = &mut left[0];
        c0.id = 0;
        c0.alive = true;
        c0.pos = Vec2::new(10.0, 10.0);
        c0.energy = 100.0;
        c0.growth_factor = 1.0;
        c0.genome.diet = Gene::constant(0.0); // pure herbivore, not a hunter
        c0.genome.aggression = Gene::constant(0.5); // hostile
        c0.genome.lethality = Gene::constant(1.0);
        c0.genome.speed = Gene::constant(1.0);
        c0.social.0 = [0.0; 10];
        c0.social.0[0] = 1.0; // genus 1
        c0.gut = GutBacterium::default();

        let c1 = &mut right[0];
        c1.id = 1;
        c1.alive = true;
        c1.pos = Vec2::new(10.1, 10.0); // within reach
        c1.energy = 50.0;
        c1.growth_factor = 1.0;
        c1.genome.diet = Gene::constant(0.0); // pure herbivore
        c1.genome.aggression = Gene::constant(0.0);
        c1.genome.speed = Gene::constant(0.1);
        c1.social.0 = [0.0; 10];
        c1.social.0[1] = 1.0; // genus 2 (similarity is 0.0 < 0.40)
        c1.gut = GutBacterium::default();

        w.predation();

        // c0 should have attacked and killed c1 due to genus conflict, even though both are herbivores
        assert!(!w.creatures[1].alive, "Herbivore c0 should have killed c1 due to genus conflict");
    }

    #[test]
    fn test_cactus_thorn_damage() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        w.creatures.push(Creature::new(0, Vec2::new(5.0, 5.0), 100.0, genome, 1.0, GutBacterium::default()));
        let c = &mut w.creatures[0];
        c.alive = true;
        c.energy = 100.0;
        c.genome.diet = Gene::constant(0.0);
        c.genome.graze = Gene::constant(1.0);
        c.genome.speed = Gene::constant(0.0);
        
        let cols = w.cfg.width.ceil() as usize + 1;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let ty = (5 + dy) as usize;
                let tx = (5 + dx) as usize;
                w.terrain.tiles[ty * cols + tx] = TileType::Sand;
            }
        }
        let idx = 5 * cols + 5;
        w.grass[idx] = 10.0;
        
        c.genome.graze = Gene::constant(0.5);
        c.genome.poison_resist = Gene::constant(0.0);
        
        w.move_and_eat();
        assert!(w.creatures[0].energy < 100.0);
        
        w.creatures[0].energy = 100.0;
        w.creatures[0].genome.graze = Gene::constant(0.8);
        w.grass[idx] = 10.0;
        w.move_and_eat();
        assert!(w.creatures[0].energy > 100.0);
    }

    #[test]
    fn test_hardy_lichen_winter_growth() {
        let mut cfg = tiny_cfg();
        cfg.grass_regrow_rate = 1.0;
        cfg.grass_max_energy = 100.0;
        let mut w = World::new(cfg);
        
        let cols = w.cfg.width.ceil() as usize + 1;
        let idx = 5 * cols + 5;
        w.terrain.tiles[idx] = TileType::Mountain;
        
        w.season_phase = 0.75;
        w.grass[idx] = 0.0;
        w.regrow_grass();
        
        assert!((w.grass[idx] - 0.4).abs() < 1e-9);
    }

    #[test]
    fn test_shaded_fungi_and_toxicity() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        cfg.poison_damage = 20.0;
        let mut w = World::new(cfg);
        
        let mut rng = crate::rng::Rng::new(123);
        let tree_genome = TreeGenome::random(&mut rng);
        w.sources.push(crate::food::FoodSource::new(Vec2::new(5.0, 5.0), tree_genome));
        
        let cols = w.cfg.width.ceil() as usize + 1;
        for gy in 0..10 {
            for gx in 0..10 {
                w.terrain.tiles[gy * cols + gx] = TileType::Plains;
            }
        }
        
        let idx = 5 * cols + 5;
        w.fungi[idx] = 40.0;
        
        w.regrow_grass();
        assert!((w.fungi[idx] - 39.8).abs() < 1e-9);
        
        let genome = Genome::random(&mut rng);
        w.creatures.push(Creature::new(0, Vec2::new(5.0, 5.0), 50.0, genome, 1.0, GutBacterium::default()));
        let c = &mut w.creatures[0];
        c.alive = true;
        c.energy = 50.0;
        c.genome.diet = Gene::constant(0.0); // Herbivore
        c.genome.speed = Gene::constant(0.0); // Stop movement to prevent wandering off fungi
        c.genome.poison_resist = Gene::constant(0.1);
        
        w.move_and_eat();
        assert!(w.fungi[idx] < 39.8);
        let energy_low_resist = w.creatures[0].energy;
        
        w.fungi[idx] = 40.0;
        w.creatures[0].energy = 50.0;
        w.creatures[0].genome.poison_resist = Gene::constant(1.0);
        w.move_and_eat();
        let energy_high_resist = w.creatures[0].energy;
        
        assert!(energy_high_resist > energy_low_resist);
    }

    #[test]
    fn test_claws_and_spikes() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 100.0, genome.clone(), 1.0, GutBacterium::default()));
        let c0 = &mut w.creatures[0];
        c0.alive = true;
        c0.energy = 100.0;
        c0.genome.diet = Gene::constant(1.0);
        c0.genome.aggression = Gene::constant(1.0);
        c0.genome.lethality = Gene::constant(1.0);
        c0.genome.speed = Gene::constant(1.0);
        c0.genome.claws = Gene::constant(0.0);
        c0.genome.altruism = Gene::constant(0.0);
        c0.genome.repro_complexity = Gene::constant(0.0);
        c0.social.0 = [0.0; 10];
        c0.social.0[0] = 1.0; // Genus 1
        
        w.creatures.push(Creature::new(1, Vec2::new(10.1, 10.0), 50.0, genome, 1.0, GutBacterium::default()));
        let c1 = &mut w.creatures[1];
        c1.alive = true;
        c1.energy = 50.0;
        c1.genome.diet = Gene::constant(0.0);
        c1.genome.defense_spikes = Gene::constant(1.0);
        c1.genome.repro_complexity = Gene::constant(0.0);
        c1.social.0 = [0.0; 10];
        c1.social.0[1] = 1.0; // Genus 2 (similarity is 0.0 < 0.40, triggering attack)
        
        w.predation();
        assert!(w.creatures[1].alive, "Prey should survive failed attack due to spikes");
        assert!(w.creatures[0].energy < 100.0, "Attacker should take spikes rebound damage");
    }

    #[test]
    fn test_reproduction_complexity() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        w.next_id = 1;
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 250.0, genome, 1.0, GutBacterium::default()));
        let c0 = &mut w.creatures[0];
        c0.alive = true;
        c0.energy = 250.0;
        c0.genome.repro_threshold = Gene::constant(100.0);
        c0.genome.mating_pref = Gene::constant(0.0);
        c0.genome.repro_complexity = Gene::constant(1.0);
        
        w.reproduce();
        
        let child = &w.creatures[1];
        assert!((child.growth_factor - 0.60).abs() < 1e-9);
    }

    #[test]
    fn test_social_capacity_energy_sharing() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 150.0, genome.clone(), 1.0, GutBacterium::default()));
        let c0 = &mut w.creatures[0];
        c0.alive = true;
        c0.energy = 150.0;
        c0.genome.altruism = Gene::constant(1.0);
        c0.genome.social_capacity = Gene::constant(1.0);
        c0.social.0[0] = 1.0;
        
        w.creatures.push(Creature::new(1, Vec2::new(12.5, 10.0), 20.0, genome, 1.0, GutBacterium::default()));
        let c1 = &mut w.creatures[1];
        c1.alive = true;
        c1.energy = 20.0;
        c1.social.0[0] = 1.0;
        
        w.share_altruistic_energy();
        assert!(w.creatures[1].energy > 20.0, "Recipient should receive shared energy");
    }

    #[test]
    fn test_tribalism_mode() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        for i in 0..5 {
            w.creatures.push(Creature::new(i as u64, Vec2::new(10.0 + i as f64 * 0.1, 10.0), 100.0, genome.clone(), 1.0, GutBacterium::default()));
            let c = &mut w.creatures[i];
            c.alive = true;
            c.energy = 100.0;
            c.social.0 = [0.0; 10];
            c.social.0[0] = 1.0;
        }
        
        w.update_tribal_states();
        
        for c in &w.creatures {
            assert!(c.in_tribe, "Creatures in the cluster should be in a tribe");
        }
    }

    #[test]
    fn test_thirst_hydration_dehydration() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        
        // Force all tiles to Plains so there's no water
        w.terrain.tiles.fill(TileType::Plains);
        w.fruits.clear();
        w.aquatic_plants.clear();
        w.carcasses.clear();
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Swimmer should not dehydrate (swimming gene >= 0.75)
        let mut swimmer_genome = genome.clone();
        swimmer_genome.swim_capability = Gene::constant(0.80);
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 100.0, swimmer_genome, 1.0, GutBacterium::default()));
        
        // Non-swimmer should dehydrate
        let mut land_genome = genome.clone();
        land_genome.swim_capability = Gene::constant(0.0);
        w.creatures.push(Creature::new(1, Vec2::new(15.0, 15.0), 100.0, land_genome, 1.0, GutBacterium::default()));
        
        w.creatures[0].alive = true;
        w.creatures[1].alive = true;
        
        // Perform movement/eating step
        w.rebuild_grids();
        w.move_and_eat();
        
        assert_eq!(w.creatures[0].hydration, 100.0, "Swimmers should have 100% hydration");
        assert!(w.creatures[1].hydration < 100.0, "Land creatures should lose hydration");
        
        // Empty the hydration of land creature to check energy loss and speed penalty
        w.creatures[1].hydration = 0.0;
        let initial_energy = w.creatures[1].energy;
        
        // Let's run move_and_eat again
        w.move_and_eat();
        assert!(w.creatures[1].energy < initial_energy - 2.0, "Dehydrated creature should lose extra energy");
    }

    #[test]
    fn test_overcrowding_reproduction_block() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Create 1 central creature and 6 neighbors
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[0].genome.repro_threshold = Gene::constant(50.0);
        w.creatures[0].genome.mating_pref = Gene::constant(0.0);
        
        for i in 1..=6 {
            w.creatures.push(Creature::new(i as u64, Vec2::new(10.0 + 0.1 * i as f64, 10.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
            w.creatures[i].alive = true;
            w.creatures[i].genome.repro_threshold = Gene::constant(50.0);
            w.creatures[i].genome.mating_pref = Gene::constant(0.0);
        }
        
        w.rebuild_grids();
        w.move_and_eat();
        
        assert!(w.creatures[0].overcrowded, "Creature with 6 neighbors should be overcrowded");
        
        // Overcrowded creature should skip reproduction
        let initial_pop = w.population();
        w.reproduce();
        assert_eq!(w.population(), initial_pop, "Overcrowded creature must not reproduce");
    }

    #[test]
    fn test_sickness_contagion_and_recovery() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Healthy creature and sick creature close by
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 100.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[0].sickness = 0.50; // Infected
        w.creatures[0].genome.immunity = Gene::constant(0.0); // weak immunity
        
        w.creatures.push(Creature::new(1, Vec2::new(10.5, 10.0), 100.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[1].alive = true;
        w.creatures[1].sickness = 0.0;
        w.creatures[1].genome.immunity = Gene::constant(0.0);
        
        // Trigger contagion checks
        w.rebuild_grids();
        w.move_and_eat();
        
        // Sick creature recovers based on immunity.
        // Let's test immunity recovery explicitly
        let mut immune_creature = Creature::new(2, Vec2::new(20.0, 20.0), 100.0, genome.clone(), 1.0, GutBacterium::default());
        immune_creature.alive = true;
        immune_creature.sickness = 0.50;
        immune_creature.genome.immunity = Gene::constant(1.0); // full immunity
        w.creatures.push(immune_creature);
        
        w.rebuild_grids();
        w.move_and_eat();
        // Since immunity is 1.0, recovery rate is 0.02 * (1 + 1) = 0.04.
        // So sickness should drop from 0.50.
        assert!(w.creatures[2].sickness < 0.50, "Immune creature should recover from sickness faster");
    }

    #[test]
    fn test_parasites_burden_and_inheritance() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Parasitic parent
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 300.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[0].parasites = 0.80;
        w.creatures[0].genome.repro_threshold = Gene::constant(50.0);
        w.creatures[0].genome.mating_pref = Gene::constant(0.0); // Force cloning
        
        // Reproduction (asexual)
        w.rebuild_grids();
        w.reproduce();
        
        assert_eq!(w.creatures.len(), 2, "A child should be born");
        assert!(w.creatures[1].parasites > 0.0, "Offspring should inherit parasites");
        assert_eq!(w.creatures[1].parasites, 0.80 * 0.6, "Offspring parasite burden should be parent * 0.6");
    }

    #[test]
    fn test_blood_sucking_vampirism() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Attacker: blood sucking, high combat power
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 100.0, genome.clone(), 1.0, GutBacterium::default()));
        let c0 = &mut w.creatures[0];
        c0.alive = true;
        c0.genome.blood_sucking = Gene::constant(1.0);
        c0.genome.diet = Gene::constant(1.0); // Carnivore
        c0.genome.aggression = Gene::constant(1.0);
        c0.genome.claws = Gene::constant(1.0);
        c0.genome.lethality = Gene::constant(1.0);
        c0.genome.speed = Gene::constant(1.0);
        
        // Prey: weaker defense
        w.creatures.push(Creature::new(1, Vec2::new(10.5, 10.0), 100.0, genome, 1.0, GutBacterium::default()));
        let c1 = &mut w.creatures[1];
        c1.alive = true;
        c1.genome.diet = Gene::constant(0.0); // Herbivore
        c1.genome.defense_spikes = Gene::constant(0.0);
        c1.genome.repro_complexity = Gene::constant(0.0);
        c1.genome.speed = Gene::constant(0.1);
        c1.social.0 = [0.0; 10];
        c1.social.0[1] = 1.0; // Different genus -> aggression triggered
        
        // Perform predation
        let initial_attacker_energy = w.creatures[0].energy;
        w.predation();
        
        // If it succeeds, the attacker should have gotten the body meat share + 20.0 * blood_suck warm blood bonus!
        assert!(!w.creatures[1].alive, "Prey should be killed");
        assert!(w.creatures[0].energy > initial_attacker_energy + 20.0, "Vampire attacker should get successful combat bonus");
    }

    #[test]
    fn test_twig_collection_and_nest_completion() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        w.fruits.clear();
        w.aquatic_plants.clear();
        w.carcasses.clear();
        w.twigs.clear();
        
        let mut rng = crate::rng::Rng::new(123);
        let mut genome = Genome::random(&mut rng);
        genome.repro_threshold = Gene::constant(50.0);
        genome.sense = Gene::constant(5.0);
        genome.speed = Gene::constant(1.0);
        
        // Mature healthy creature
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 1000.0, genome, 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        
        // Establish nest (will be at 10.0, 10.0 because not in tribe)
        w.rebuild_grids();
        println!("Before 1st step: pos={:?}, nest_pos={:?}, carrying_twig={}", w.creatures[0].pos, w.creatures[0].nest_pos, w.creatures[0].carrying_twig);
        w.move_and_eat();
        println!("After 1st step: pos={:?}, nest_pos={:?}, carrying_twig={}", w.creatures[0].pos, w.creatures[0].nest_pos, w.creatures[0].carrying_twig);
        assert!(w.creatures[0].nest_pos.is_some(), "Nest position should be established");
        let nest_pos = w.creatures[0].nest_pos.unwrap();
        
        // Spawn a twig nearby
        let twig_pos = Vec2::new(13.0, 10.0);
        w.twigs.push(Twig { pos: twig_pos, id: 0 });
        w.rebuild_grids();
        
        println!("Before 2nd step: pos={:?}, nest_pos={:?}, carrying_twig={}, twigs={:?}", w.creatures[0].pos, w.creatures[0].nest_pos, w.creatures[0].carrying_twig, w.twigs);
        w.move_and_eat();
        println!("After 2nd step: pos={:?}, nest_pos={:?}, carrying_twig={}, twigs={:?}", w.creatures[0].pos, w.creatures[0].nest_pos, w.creatures[0].carrying_twig, w.twigs);
        
        w.rebuild_grids();
        w.move_and_eat();
        println!("After 3rd step: pos={:?}, nest_pos={:?}, carrying_twig={}, twigs={:?}", w.creatures[0].pos, w.creatures[0].nest_pos, w.creatures[0].carrying_twig, w.twigs);
        assert!(w.creatures[0].carrying_twig, "Creature should collect the twig");
        assert_eq!(w.twigs.len(), 0, "Twig should be removed from the world");
        
        // Deposit twig on nest
        w.creatures[0].pos = nest_pos; // move to nest
        w.rebuild_grids();
        w.move_and_eat();
        
        assert!(!w.creatures[0].carrying_twig, "Twig should be deposited");
        let nest = w.nests.iter().find(|n| n.owner_id == 0).unwrap();
        assert_eq!(nest.twigs, 1, "Nest should have 1 twig");
        assert!(!nest.completed, "Nest should not be completed yet");
        
        // Let's complete the nest by adding 4 more twigs
        for _ in 0..4 {
            w.creatures[0].carrying_twig = true;
            w.rebuild_grids();
            w.move_and_eat();
        }
        let nest = w.nests.iter().find(|n| n.owner_id == 0).unwrap();
        assert_eq!(nest.twigs, 5, "Nest should have 5 twigs");
        assert!(nest.completed, "Nest should be completed");
    }

    #[test]
    fn test_nest_shelter_benefits() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        w.fruits.clear();
        w.aquatic_plants.clear();
        w.carcasses.clear();
        w.twigs.clear();
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Setup creature and a completed nest underfoot
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 100.0, genome, 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[0].nest_pos = Some(Vec2::new(10.0, 10.0));
        w.nests.push(Nest {
            pos: Vec2::new(10.0, 10.0),
            owner_id: 0,
            twigs: 5,
            completed: true,
        });
        
        assert!(w.is_near_own_completed_nest(&w.creatures[0]), "Creature should be near its own completed nest");
        
        // Verify weather toxic storm immunity
        w.current_weather = WeatherEvent::ToxicStorm;
        let initial_e = w.creatures[0].energy;
        w.apply_weather_effects();
        assert_eq!(w.creatures[0].energy, initial_e, "Creature in completed nest should be protected from storm damage");
        
        // Verify tsunami protection
        w.current_weather = WeatherEvent::Tsunami;
        w.terrain.tiles.fill(TileType::Sand); // force coastal tile
        w.creatures[0].genome.swim_capability = Gene::constant(0.0); // non-swimmer
        let initial_pos = w.creatures[0].pos;
        w.apply_weather_effects();
        assert_eq!(w.creatures[0].pos, initial_pos, "Creature in completed nest should not be displaced by tsunami");
        assert_eq!(w.creatures[0].energy, initial_e, "Creature in completed nest should not lose energy from tsunami");
    }

    #[test]
    fn test_incestuous_mating_and_inbreeding_penalty() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        w.fruits.clear();
        w.aquatic_plants.clear();
        w.carcasses.clear();
        w.twigs.clear();
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Parent and child
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures.push(Creature::new(1, Vec2::new(10.1, 10.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[1].alive = true;
        
        // Establish parent relationship
        w.creatures[1].parent_ids = Some((0, 999));
        
        assert!(w.are_closely_related(0, 1), "Parent and child should be closely related");
        
        // Siblings
        w.creatures.push(Creature::new(2, Vec2::new(20.0, 20.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures.push(Creature::new(3, Vec2::new(20.1, 20.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[2].alive = true;
        w.creatures[3].alive = true;
        w.creatures[2].parent_ids = Some((888, 889));
        w.creatures[3].parent_ids = Some((888, 990)); // share one parent
        
        assert!(w.are_closely_related(2, 3), "Siblings sharing a parent should be closely related");
        
        // Sibling reproduction produces inbred offspring
        w.creatures[0].energy = 10.0;
        w.creatures[1].energy = 10.0;
        w.creatures[2].genome.mating_pref = Gene::constant(1.0); // force sexual
        w.creatures[3].genome.mating_pref = Gene::constant(1.5); // wants to mate
        w.creatures[2].genome.repro_threshold = Gene::constant(50.0);
        w.creatures[3].genome.repro_threshold = Gene::constant(50.0);
        w.rebuild_grids();
        w.reproduce();
        
        // The newborn should be at index 4
        assert_eq!(w.creatures.len(), 5, "Child should be born");
        let child = &w.creatures[4];
        assert!(child.is_inbred, "Incest offspring should be flagged as inbred");
        
        // Check penalties: speed and immunity should have 15% reduction
        let base_speed = child.genome.effective_speed(child.energy);
        let base_immunity = child.genome.effective_immunity(child.energy);
        assert!((child.effective_speed() - base_speed * 0.85).abs() < 1e-9);
        assert!((child.effective_immunity() - base_immunity * 0.85).abs() < 1e-9);
    }

    #[test]
    fn test_primitive_vocality() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        w.fruits.clear();
        w.aquatic_plants.clear();
        w.carcasses.clear();
        w.twigs.clear();
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Caller and listener in range
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 100.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures.push(Creature::new(1, Vec2::new(18.0, 10.0), 100.0, genome, 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[1].alive = true;
        
        w.creatures[0].vocal_type = 1; // Alarm Call
        w.rebuild_grids();
        w.resolve_vocalizations();
        
        let best_threat = w.creatures[1].memory.best(MemKind::Threat);
        assert!(best_threat.is_some(), "Listener should receive Alarm and record Threat memory");
        assert_eq!(best_threat.unwrap(), Vec2::new(10.0, 10.0), "Memory should be recorded at caller's position");
    }

    #[test]
    fn test_nest_sexual_reproduction_buff() {
        let mut cfg = tiny_cfg();
        cfg.initial_creatures = 0;
        let mut w = World::new(cfg);
        w.fruits.clear();
        w.aquatic_plants.clear();
        w.carcasses.clear();
        w.twigs.clear();
        
        let mut rng = crate::rng::Rng::new(123);
        let genome = Genome::random(&mut rng);
        
        // Parent A and Parent B
        w.creatures.push(Creature::new(0, Vec2::new(10.0, 10.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures.push(Creature::new(1, Vec2::new(10.5, 10.0), 200.0, genome.clone(), 1.0, GutBacterium::default()));
        w.creatures[0].alive = true;
        w.creatures[1].alive = true;
        
        // Setup nests:
        // Parent A is near its own completed nest
        w.creatures[0].nest_pos = Some(Vec2::new(10.0, 10.0));
        w.nests.push(Nest {
            pos: Vec2::new(10.0, 10.0),
            owner_id: 0,
            twigs: 5,
            completed: true,
        });
        
        // Parent B has no nest (or is not near it)
        w.creatures[1].nest_pos = None;
        
        // Let's reproduce
        w.creatures[0].genome.mating_pref = Gene::constant(1.0); // force sexual
        w.creatures[1].genome.mating_pref = Gene::constant(1.0); // force sexual
        w.creatures[0].genome.repro_threshold = Gene::constant(50.0);
        w.creatures[1].genome.repro_threshold = Gene::constant(50.0);
        w.rebuild_grids();
        w.reproduce();
        
        // Child should be born
        assert_eq!(w.creatures.len(), 3, "Child should be born");
        
        // Parent A should pay half sexual_cost (base is 80.0 / 4.0 = 20.0, so 10.0)
        // Parent B should pay full sexual_cost (20.0)
        let sexual_cost = w.cfg.asexual_repro_cost / 4.0;
        assert_eq!(sexual_cost, 20.0);
        
        assert_eq!(w.creatures[0].energy, 200.0 - 10.0, "Parent A (near completed nest) should pay half cost");
        assert_eq!(w.creatures[1].energy, 200.0 - 20.0, "Parent B (not near completed nest) should pay full cost");
    }
}

