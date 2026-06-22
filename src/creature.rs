//! The creatures: mobile agents driven by their genome, energy, memory, and
//! social embedding.

use crate::genome::{DietClass, Genome};
use crate::geometry::Vec2;
use crate::gut::GutBacterium;
use crate::memory::MemoryBuffer;
use crate::social::SocialVec;

#[derive(Clone, Debug)]
pub struct Creature {
    pub id: u64,
    pub pos: Vec2,
    pub energy: f64,
    /// Number of end-of-cycle death rolls already survived. Drives the
    /// escalating death probability (30% + 5% per cycle survived).
    pub cycles_survived: u32,
    /// Ticks lived (for stats / display).
    pub age_ticks: u64,
    pub genome: Genome,
    pub alive: bool,
    /// Guard so a creature reproduces at most once per tick.
    pub reproduced_this_tick: bool,
    /// Set once this creature's body has been turned into a carcass, so the
    /// remains aren't spawned twice (e.g. a predator already left the leftovers).
    pub carcass_spawned: bool,

    /// Episodic memory: the last N sensory observations (food, threats, mates,
    /// carcasses) used to navigate when nothing is currently in sense range.
    pub memory: MemoryBuffer,

    /// Social embedding vector: a compact summary of the creature's social
    /// neighbourhood, updated each tick from proximity contacts.
    /// Used for kin recognition (reduces predation between similar creatures)
    /// and mate preference (similar vectors pair more readily).
    pub social: SocialVec,

    /// Age / growth factor: 0.25 at birth, 0.5 at division, 1.0 when fully grown.
    /// Scales physical size, metabolism, and combat power.
    pub growth_factor: f64,

    /// Running estimate of Y-directional food source migration drift.
    pub food_drift_est: f64,

    /// Gut bacterium: defines plant/meat eating fit and mood/aggression.
    /// Gut bacterium: defines plant/meat eating fit and mood/aggression.
    pub gut: GutBacterium,

    /// Whether this creature currently belongs to a herding tribe.
    pub in_tribe: bool,
    pub overcrowded: bool,

    pub sickness: f64,
    pub parasites: f64,
    pub hydration: f64,

    pub nest_pos: Option<Vec2>,
    pub carrying_twig: bool,
    pub parent_ids: Option<(u64, u64)>,
    pub is_inbred: bool,
    pub vocal_type: u8,
}

impl Creature {
    pub fn new(id: u64, pos: Vec2, energy: f64, genome: Genome, growth_factor: f64, gut: GutBacterium) -> Self {
        // Initialise social identity from genome traits so creatures start
        // with a biologically-meaningful embedding before any social contact.
        let social = SocialVec::from_genome(
            genome.diet.a,
            genome.size.a,
            genome.speed.a,
            genome.mating_pref.a,
            genome.temp_optimum.a,
            genome.aggression.a,
            genome.lethality.a,
            genome.feed_efficiency.a,
            genome.sociability.a,
            genome.altruism.a,
        );
        Creature {
            id,
            pos,
            energy,
            cycles_survived: 0,
            age_ticks: 0,
            genome,
            alive: true,
            reproduced_this_tick: false,
            carcass_spawned: false,
            memory: MemoryBuffer::new(),
            social,
            growth_factor,
            food_drift_est: 0.0,
            gut,
            in_tribe: false,
            overcrowded: false,
            sickness: 0.0,
            parasites: 0.0,
            hydration: 100.0,
            nest_pos: None,
            carrying_twig: false,
            parent_ids: None,
            is_inbred: false,
            vocal_type: 0,
        }
    }

    /// Combat power incorporating gut bacterium mood modifications.
    pub fn combat_power(&self) -> f64 {
        let base_power = self.genome.combat_power(self.energy, self.growth_factor);
        let claws_boost = 1.0 + self.genome.effective_claws(self.energy) * 0.5;
        let tribe_mult = if self.in_tribe { 1.30 } else { 1.0 };
        // Gut bacterium mood aggression shifts combat power by up to 25%
        let mood_mult = 1.0 + self.gut.mood_aggression * 0.5; // -25% to +25%
        base_power * claws_boost * tribe_mult * mood_mult.max(0.1)
    }

    /// Edible energy this creature's body yields when it dies.
    pub fn body_energy(&self, body_factor: f64) -> f64 {
        self.genome.size.a * body_factor * self.growth_factor + self.energy.max(0.0) * 0.5
    }

    /// Whether this creature will eat meat from prey or carcasses at all.
    /// Anything with some carnivory (omnivores included) scavenges; pure
    /// herbivores do not. Uses the energy-contextual diet value.
    pub fn eats_meat(&self) -> bool {
        self.genome.effective_diet(self.energy) >= 0.3
    }

    /// Whether this creature currently has the energy to attempt reproduction.
    pub fn wants_to_reproduce(&self, repro_cost: f64, is_near_completed_nest: bool) -> bool {
        let thresh = if is_near_completed_nest {
            self.genome.repro_threshold.a * 0.8
        } else {
            self.genome.repro_threshold.a
        };
        self.alive
            && self.growth_factor >= 1.0 // Must be fully grown to reproduce
            && !self.reproduced_this_tick
            && self.energy >= thresh
            // Must be able to afford the full (asexual) cost and survive it.
            && self.energy > repro_cost + 1.0
    }

    /// Record a food location in memory and learn its spatial drift trend.
    pub fn record_food_memory(&mut self, pos: Vec2, val: f32) {
        if let Some(avg_y) = self.memory.average_y(crate::memory::MemKind::Food) {
            let diff_y = pos.y - avg_y;
            self.food_drift_est = self.food_drift_est * 0.95 + diff_y * 0.05;
        }
        self.memory.record(crate::memory::MemKind::Food, pos, val);
    }

    /// Diet class based on the current expressed diet (energy-contextual).
    pub fn diet_class(&self) -> DietClass {
        self.genome.diet_class_at(self.energy)
    }

    /// Whether this creature pursues prey (energy-contextual, influenced by gut mood).
    pub fn hunts(&self) -> bool {
        let mut diet = self.genome.effective_diet(self.energy);
        // Aggressive gut mood makes them lean more toward carnivory/hunting
        if self.gut.mood_aggression > 0.0 {
            diet += self.gut.mood_aggression * 0.2;
        } else if self.gut.mood_aggression < 0.0 {
            diet += self.gut.mood_aggression * 0.1; // calmer mood reduces hunting slightly
        }
        diet.clamp(0.0, 1.0) >= 0.5
    }

    /// Display glyph: letter by diet class, case by mating preference
    /// (UPPER = asexual-leaning, lower = sexual-leaning).
    pub fn symbol(&self) -> char {
        let l = self.diet_class().letter();
        if self.genome.mating_pref.a >= 0.5 {
            l.to_ascii_lowercase()
        } else {
            l
        }
    }

    pub fn effective_speed(&self) -> f64 {
        let mut speed = self.genome.effective_speed(self.energy);
        if self.is_inbred {
            speed *= 0.85;
        }
        speed
    }

    pub fn effective_immunity(&self) -> f64 {
        let mut immunity = self.genome.effective_immunity(self.energy);
        if self.is_inbred {
            immunity *= 0.85;
        }
        immunity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{DietClass, Gene};
    use crate::rng::Rng;

    fn sample(energy: f64) -> Creature {
        let mut rng = Rng::new(1);
        let mut g = Genome::random(&mut rng);
        g.repro_threshold = Gene::constant(120.0);
        Creature::new(1, Vec2::zero(), energy, g, 1.0, GutBacterium::default())
    }

    #[test]
    fn wont_reproduce_below_threshold() {
        let c = sample(80.0);
        assert!(!c.wants_to_reproduce(100.0, false));
    }

    #[test]
    fn reproduces_when_rich_enough() {
        let c = sample(200.0);
        assert!(c.wants_to_reproduce(100.0, false));
    }

    #[test]
    fn symbol_reflects_diet_class() {
        let mut c = sample(200.0);
        c.genome.diet = Gene::constant(0.0);
        c.genome.mating_pref = Gene::constant(0.0);
        assert_eq!(c.symbol(), 'H');
        c.genome.diet = Gene::constant(1.0);
        assert_eq!(c.symbol(), 'C');
        c.genome.diet = Gene::constant(0.5);
        assert_eq!(c.diet_class(), DietClass::Omnivore);
        c.genome.mating_pref = Gene::constant(0.9);
        assert_eq!(c.symbol(), 'o');
    }

    #[test]
    fn memory_initializes_empty() {
        let c = sample(100.0);
        assert!(c.memory.best(crate::memory::MemKind::Food).is_none());
    }

    #[test]
    fn social_vec_initializes_from_genome() {
        let c = sample(100.0);
        // Social vector should be non-zero (derived from genome traits).
        let sum: f32 = c.social.0.iter().sum();
        assert!(sum > 0.0, "social vector should be non-zero");
    }
}
