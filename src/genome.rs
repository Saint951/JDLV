//! Heritable traits and how they mutate / recombine.
//!
//! ## Polynomial genes
//!
//! Every trait is now encoded as a **degree-2 polynomial** over a normalised
//! "context" variable `x` (the creature's energy state, centred at 0.0 when
//! energy equals the reproductive threshold):
//!
//! ```text
//! trait_value(x) = a + b·x + c·x²
//! ```
//!
//! When `b = c = 0` this collapses to a constant — identical to the old
//! scalar behaviour. Selection pressure on `b` and `c` lets lineages evolve
//! **phenotypic plasticity**: a starving animal (x < 0) might become more
//! aggressive or omnivorous, while a well-fed one (x > 0) invests in
//! reproduction. Nothing is hard-coded; plasticity itself is heritable.
//!
//! ## Coefficients mutate at different rates
//! - `a` (baseline): full mutation magnitude — shapes the population average.
//! - `b` (linear slope): ¼ magnitude — slower drift toward plasticity.
//! - `c` (quadratic curve): ⅛ magnitude — evolves most conservatively.
//!
//! ## Stats and display
//! For population statistics and the ASCII/web renderer, the *baseline*
//! value `gene.a` (= `gene.evaluate(0.0)`) is used; it represents the
//! creature's trait at neutral energy and is the most interpretable number.

use crate::rng::Rng;

// ─────────────────────────────────────────────────────────────────────────────
// Gene — the atomic unit of the genome
// ─────────────────────────────────────────────────────────────────────────────

/// A single heritable trait encoded as a quadratic polynomial.
///
/// `evaluate(x)` returns the expressed trait value given the creature's
/// normalised context `x` (see [`Genome::energy_x`]).
#[derive(Clone, Copy, Debug)]
pub struct Gene {
    /// Constant term — the trait value at neutral context (`x = 0`).
    pub a: f64,
    /// Linear plasticity coefficient.
    pub b: f64,
    /// Quadratic plasticity coefficient.
    pub c: f64,
}

impl Gene {
    /// A constant gene (no plasticity): equivalent to the old scalar trait.
    pub fn constant(v: f64) -> Self {
        Gene { a: v, b: 0.0, c: 0.0 }
    }

    /// Random gene: baseline in `[lo, hi]`, small random plasticity.
    pub fn random(rng: &mut Rng, lo: f64, hi: f64) -> Self {
        Gene {
            a: rng.range_f64(lo, hi),
            b: rng.gaussian() * (hi - lo) * 0.02,
            c: rng.gaussian() * (hi - lo) * 0.01,
        }
    }

    /// Evaluate the polynomial at context `x`.
    #[inline]
    pub fn evaluate(&self, x: f64) -> f64 {
        self.a + self.b * x + self.c * x * x
    }

    /// Mutate: jitter coefficients by independent Gaussian noise.
    ///
    /// `scale_a` is the mutation magnitude for the baseline; `b` and `c` get
    /// ¼ and ⅛ of that respectively, so plasticity evolves more conservatively.
    pub fn mutate(&self, rng: &mut Rng, scale_a: f64) -> Self {
        Gene {
            a: self.a + rng.gaussian() * scale_a,
            b: self.b + rng.gaussian() * scale_a * 0.25,
            c: self.c + rng.gaussian() * scale_a * 0.125,
        }
    }

    /// Blend two parental genes (weighted average), then mutate the result.
    pub fn crossover(p: &Gene, q: &Gene, rng: &mut Rng, scale_a: f64) -> Self {
        let t = rng.range_f64(0.3, 0.7);
        Gene {
            a: p.a * t + q.a * (1.0 - t),
            b: p.b * t + q.b * (1.0 - t),
            c: p.c * t + q.c * (1.0 - t),
        }
        .mutate(rng, scale_a)
    }

    /// Clamp the baseline `a` to `[lo, hi]` and the plasticity coefficients
    /// to a symmetric range proportional to the trait span.
    pub fn clamped(self, lo: f64, hi: f64) -> Self {
        let span = (hi - lo).max(1e-6);
        Gene {
            a: self.a.clamp(lo, hi),
            b: self.b.clamp(-span * 0.5, span * 0.5),
            c: self.c.clamp(-span * 0.25, span * 0.25),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DietClass
// ─────────────────────────────────────────────────────────────────────────────

/// Coarse dietary class derived from the continuous `diet` gene, used for
/// display and the Herbivore/Omnivore/Carnivore population graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DietClass {
    Herbivore,
    Omnivore,
    Carnivore,
}

impl DietClass {
    pub fn from_diet(diet: f64) -> DietClass {
        if diet < 1.0 / 3.0 {
            DietClass::Herbivore
        } else if diet > 2.0 / 3.0 {
            DietClass::Carnivore
        } else {
            DietClass::Omnivore
        }
    }

    /// Base display letter (the renderer chooses upper/lower case from the
    /// mating-preference gene).
    pub fn letter(self) -> char {
        match self {
            DietClass::Herbivore => 'H',
            DietClass::Omnivore  => 'O',
            DietClass::Carnivore => 'C',
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Genome
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Genome {
    /// Max movement distance per tick; costs energy quadratically.
    pub speed: Gene,
    /// Perception radius for finding food / mates / threats.
    pub sense: Gene,
    /// Body size: bigger wins fights but costs more energy to run.
    pub size: Gene,
    /// Baseline energy burned per tick (independent of movement).
    pub metabolism: Gene,
    /// Diet position: 0.0 = pure herbivore, 1.0 = pure carnivore.
    /// Plasticity here lets starving creatures shift toward opportunism.
    pub diet: Gene,
    /// Willingness to commit to an attack, 0.0..1.0.
    pub aggression: Gene,
    /// Energy at which the creature attempts to reproduce.
    pub repro_threshold: Gene,
    /// Bias toward sexual reproduction (seek a mate) vs asexual self-cloning.
    /// 0.0 = always clone, 1.0 = always seek a mate.
    pub mating_pref: Gene,
    /// Preferred temperature; deviation from the local climate costs energy.
    pub temp_optimum: Gene,
    /// Climate hardiness 0.0..1.0: widens the comfortable band but raises
    /// baseline metabolism.
    pub temp_tolerance: Gene,
    /// Resistance to fruit toxins, 0.0..1.0 (carries a small upkeep cost).
    pub poison_resist: Gene,
    /// Carnivore killing power; high lethality lands kills against tough prey
    /// but leaves bigger carcasses for scavengers.
    pub lethality: Gene,
    /// How thoroughly a kill/carcass is consumed; low efficiency wastes meat.
    pub feed_efficiency: Gene,
    /// Aquatic adaptation: reduces water movement cost; ≥ 0.25 unlocks deep water.
    pub swim_capability: Gene,
    /// High-altitude adaptation: reduces mountain movement cost and cold penalty.
    pub climb_capability: Gene,
    /// Vision grid half-extent (1..=5, odd → 3×3..11×11 grid). Wider vision
    /// costs more energy to maintain.
    pub vision_size: Gene,
    /// Flocking preference: positive = attract to similar neighbors, negative = repel.
    pub sociability: Gene,
    /// Sharing behavior: positive = altruist, negative = egoist.
    pub altruism: Gene,
    /// Ability to graze grass on land tiles (useful for herbivores).
    pub graze: Gene,
    /// Offensive claws strength.
    pub claws: Gene,
    /// Defensive spikes/armor strength.
    pub defense_spikes: Gene,
    /// Reproduction gestation/complexity.
    pub repro_complexity: Gene,
    /// Social range/sharing capacity.
    pub social_capacity: Gene,
    /// Disease and parasite resistance.
    pub immunity: Gene,
    /// Blood sucking behavior/vampirism.
    pub blood_sucking: Gene,
}

impl Genome {
    // ─── context ────────────────────────────────────────────────────────────

    /// Normalised energy context `x` used to evaluate all polynomial genes.
    ///
    /// `x = 0` at the reproductive threshold (neutral state),
    /// `x > 0` when well-fed, `x < 0` when hungry.
    /// Clamped to `[-1.5, 1.5]` so the quadratic term stays bounded.
    #[inline]
    pub fn energy_x(&self, energy: f64) -> f64 {
        let ref_e = self.repro_threshold.a.max(50.0);
        ((energy / ref_e) - 1.0).clamp(-1.5, 1.5)
    }

    // ─── effective trait accessors ───────────────────────────────────────────

    /// Expressed diet at this energy level, clamped to `[0, 1]`.
    #[inline]
    pub fn effective_diet(&self, energy: f64) -> f64 {
        self.diet.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    /// Expressed speed at this energy level.
    #[inline]
    pub fn effective_speed(&self, energy: f64) -> f64 {
        self.speed.evaluate(self.energy_x(energy)).clamp(0.2, 4.0)
    }

    /// Expressed sense radius at this energy level.
    #[inline]
    pub fn effective_sense(&self, energy: f64) -> f64 {
        self.sense.evaluate(self.energy_x(energy)).clamp(1.0, 25.0)
    }

    /// Expressed size at this energy level.
    #[inline]
    pub fn effective_size(&self, energy: f64) -> f64 {
        self.size.evaluate(self.energy_x(energy)).clamp(0.4, 3.0)
    }

    /// Expressed aggression at this energy level.
    #[inline]
    pub fn effective_aggression(&self, energy: f64) -> f64 {
        self.aggression.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    /// Expressed swim capability at this energy level.
    #[inline]
    pub fn effective_swim(&self, energy: f64) -> f64 {
        self.swim_capability.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    /// Expressed climb capability at this energy level.
    #[inline]
    pub fn effective_climb(&self, energy: f64) -> f64 {
        self.climb_capability.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    /// Vision grid half-extent (1..=5).
    pub fn effective_vision_half(&self, energy: f64) -> usize {
        (self.vision_size.evaluate(self.energy_x(energy)).round() as usize)
            .clamp(1, 5)
    }

    #[inline]
    pub fn effective_sociability(&self, energy: f64) -> f64 {
        self.sociability.evaluate(self.energy_x(energy)).clamp(-1.0, 1.0)
    }

    #[inline]
    pub fn effective_altruism(&self, energy: f64) -> f64 {
        self.altruism.evaluate(self.energy_x(energy)).clamp(-1.0, 1.0)
    }

    #[inline]
    pub fn effective_graze(&self, energy: f64) -> f64 {
        self.graze.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn effective_claws(&self, energy: f64) -> f64 {
        self.claws.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn effective_defense_spikes(&self, energy: f64) -> f64 {
        self.defense_spikes.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn effective_repro_complexity(&self, energy: f64) -> f64 {
        self.repro_complexity.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn effective_social_capacity(&self, energy: f64) -> f64 {
        self.social_capacity.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn effective_immunity(&self, energy: f64) -> f64 {
        self.immunity.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn effective_blood_sucking(&self, energy: f64) -> f64 {
        self.blood_sucking.evaluate(self.energy_x(energy)).clamp(0.0, 1.0)
    }

    // ─── derived trait logic ─────────────────────────────────────────────────

    pub fn diet_class_at(&self, energy: f64) -> DietClass {
        DietClass::from_diet(self.effective_diet(energy))
    }

    /// Fruit-energy multiplier — exactly `(1 - diet)` per the spec.
    pub fn plant_efficiency(&self, energy: f64) -> f64 {
        (1.0 - self.effective_diet(energy)).clamp(0.0, 1.0)
    }

    /// Meat-energy multiplier — exactly `diet` per the spec.
    pub fn meat_efficiency(&self, energy: f64) -> f64 {
        self.effective_diet(energy).clamp(0.0, 1.0)
    }

    /// Whether this creature pursues prey (vs foraging for fruit).
    pub fn hunts(&self, energy: f64) -> bool {
        self.effective_diet(energy) >= 0.5
    }

    /// Combat power when fighting another creature.
    pub fn combat_power(&self, energy: f64, growth_factor: f64) -> f64 {
        let x   = self.energy_x(energy);
        let sz  = self.size.evaluate(x).clamp(0.4, 3.0) * growth_factor;
        let agg = self.aggression.evaluate(x).clamp(0.0, 1.0);
        let lth = self.lethality.evaluate(x).clamp(0.0, 1.0);
        sz * (1.0 + agg) * (1.0 + lth) * (1.0 + energy / 200.0)
    }

    /// Baseline energy burned this tick from metabolism, movement and upkeep.
    pub fn energy_cost(&self, moved: f64, energy: f64, growth_factor: f64) -> f64 {
        let x        = self.energy_x(energy);
        let meta     = self.metabolism.evaluate(x).clamp(0.1, 2.0);
        let sz       = self.size.evaluate(x).clamp(0.4, 3.0) * growth_factor;
        let sense    = self.sense.evaluate(x).clamp(1.0, 25.0);
        let temp_tol = self.temp_tolerance.evaluate(x).clamp(0.0, 1.0);
        let p_res    = self.poison_resist.evaluate(x).clamp(0.0, 1.0);
        let leth     = self.lethality.evaluate(x).clamp(0.0, 1.0);
        let swim     = self.swim_capability.evaluate(x).clamp(0.0, 1.0);
        let climb    = self.climb_capability.evaluate(x).clamp(0.0, 1.0);
        let vis      = self.vision_size.evaluate(x).clamp(1.0, 5.0);

        let base     = meta * sz;
        let movement = 0.12 * sz * moved * moved;
        let sensing  = 0.02 * sense;
        let upkeep   = 0.05 * temp_tol + 0.04 * p_res + 0.03 * leth
            + 0.02 * swim + 0.02 * climb
            + 0.01 * vis; // wider vision has a small cost
        base + movement + sensing + upkeep
    }

    /// Extra energy cost from being outside the creature's thermal comfort zone.
    pub fn climate_cost(&self, local_temp: f64, penalty: f64, energy: f64) -> f64 {
        let x        = self.energy_x(energy);
        let temp_opt = self.temp_optimum.evaluate(x).clamp(-1.0, 2.0);
        let temp_tol = self.temp_tolerance.evaluate(x).clamp(0.0, 1.0);
        let dev      = (local_temp - temp_opt).abs();
        let comfort  = 0.35 + 0.6 * temp_tol;
        penalty * (dev - comfort).max(0.0)
    }

    // ─── random initial genome ───────────────────────────────────────────────

    /// A random viable genome for seeding the initial population.
    pub fn random(rng: &mut Rng) -> Self {
        Genome {
            speed:          Gene::random(rng, 0.6, 2.2),
            sense:          Gene::random(rng, 5.0, 14.0),
            size:           Gene::random(rng, 0.7, 1.6),
            metabolism:     Gene::random(rng, 0.3, 0.9),
            diet:           Gene::random(rng, 0.0, 1.0),
            aggression:     Gene::random(rng, 0.0, 1.0),
            repro_threshold:Gene::random(rng, 100.0, 150.0),
            mating_pref:    Gene::random(rng, 0.0, 1.0),
            temp_optimum:   Gene::random(rng, 0.0, 1.0),
            temp_tolerance: Gene::random(rng, 0.0, 0.5),
            poison_resist:  Gene::random(rng, 0.0, 0.2),
            lethality:      Gene::random(rng, 0.0, 1.0),
            feed_efficiency:Gene::random(rng, 0.4, 0.9),
            swim_capability:Gene::random(rng, 0.0, 0.3),
            climb_capability:Gene::random(rng, 0.0, 0.3),
            vision_size:    Gene::random(rng, 1.0, 3.0),
            sociability:    Gene::random(rng, -0.6, 0.8),
            altruism:       Gene::random(rng, -0.5, 0.5),
            graze:          Gene::random(rng, 0.0, 1.0),
            claws:          Gene::random(rng, 0.0, 0.3),
            defense_spikes: Gene::random(rng, 0.0, 0.3),
            repro_complexity:Gene::random(rng, 0.0, 0.4),
            social_capacity:Gene::random(rng, 0.0, 0.4),
            immunity:       Gene::random(rng, 0.1, 0.4),
            blood_sucking:  Gene::random(rng, 0.0, 0.2),
        }
    }

    // ─── clamp all genes to valid ranges ─────────────────────────────────────

    fn clamp_all(&mut self) {
        self.speed           = self.speed.clamped(0.2, 4.0);
        self.sense           = self.sense.clamped(1.0, 25.0);
        self.size            = self.size.clamped(0.4, 3.0);
        self.metabolism      = self.metabolism.clamped(0.1, 2.0);
        self.diet            = self.diet.clamped(0.0, 1.0);
        self.aggression      = self.aggression.clamped(0.0, 1.0);
        self.repro_threshold = self.repro_threshold.clamped(85.0, 200.0);
        self.mating_pref     = self.mating_pref.clamped(0.0, 1.0);
        self.temp_optimum    = self.temp_optimum.clamped(-1.0, 2.0);
        self.temp_tolerance  = self.temp_tolerance.clamped(0.0, 1.0);
        self.poison_resist   = self.poison_resist.clamped(0.0, 1.0);
        self.lethality       = self.lethality.clamped(0.0, 1.0);
        self.feed_efficiency = self.feed_efficiency.clamped(0.1, 1.0);
        self.swim_capability = self.swim_capability.clamped(0.0, 1.0);
        self.climb_capability= self.climb_capability.clamped(0.0, 1.0);
        self.vision_size     = self.vision_size.clamped(1.0, 5.0);
        self.sociability     = self.sociability.clamped(-1.0, 1.0);
        self.altruism        = self.altruism.clamped(-1.0, 1.0);
        self.graze           = self.graze.clamped(0.0, 1.0);
        self.claws           = self.claws.clamped(0.0, 1.0);
        self.defense_spikes  = self.defense_spikes.clamped(0.0, 1.0);
        self.repro_complexity= self.repro_complexity.clamped(0.0, 1.0);
        self.social_capacity = self.social_capacity.clamped(0.0, 1.0);
        self.immunity        = self.immunity.clamped(0.0, 1.0);
        self.blood_sucking   = self.blood_sucking.clamped(0.0, 1.0);
    }

    // ─── mutation ────────────────────────────────────────────────────────────

    /// Asexual clone: very small jitter on every gene baseline and even smaller
    /// jitter on plasticity coefficients.
    pub fn mutated_low_rate(&self, rng: &mut Rng) -> Genome {
        let m = 0.04;
        let mut g = *self;
        g.speed           = g.speed.mutate(rng, m * 0.5);
        g.sense           = g.sense.mutate(rng, m * 3.0);
        g.size            = g.size.mutate(rng, m * 0.4);
        g.metabolism      = g.metabolism.mutate(rng, m * 0.2);
        g.diet            = g.diet.mutate(rng, m * 0.3);
        g.aggression      = g.aggression.mutate(rng, m * 0.3);
        g.repro_threshold = g.repro_threshold.mutate(rng, m * 12.0);
        g.mating_pref     = g.mating_pref.mutate(rng, m * 0.3);
        g.temp_optimum    = g.temp_optimum.mutate(rng, m * 0.3);
        g.temp_tolerance  = g.temp_tolerance.mutate(rng, m * 0.2);
        g.poison_resist   = g.poison_resist.mutate(rng, m * 0.2);
        g.lethality       = g.lethality.mutate(rng, m * 0.3);
        g.feed_efficiency = g.feed_efficiency.mutate(rng, m * 0.2);
        g.swim_capability = g.swim_capability.mutate(rng, m * 0.2);
        g.climb_capability= g.climb_capability.mutate(rng, m * 0.2);
        g.vision_size     = g.vision_size.mutate(rng, m * 0.5);
        g.sociability     = g.sociability.mutate(rng, m * 0.3);
        g.altruism        = g.altruism.mutate(rng, m * 0.3);
        g.graze           = g.graze.mutate(rng, m * 0.3);
        g.claws           = g.claws.mutate(rng, m * 0.3);
        g.defense_spikes  = g.defense_spikes.mutate(rng, m * 0.3);
        g.repro_complexity= g.repro_complexity.mutate(rng, m * 0.3);
        g.social_capacity = g.social_capacity.mutate(rng, m * 0.3);
        g.immunity        = g.immunity.mutate(rng, m * 0.3);
        g.blood_sucking   = g.blood_sucking.mutate(rng, m * 0.3);

        // Diet transition resistance
        let class_before = DietClass::from_diet(self.effective_diet(150.0));
        let class_after  = DietClass::from_diet(g.effective_diet(150.0));
        if class_before != class_after && rng.chance(0.85) {
            g.diet = self.diet;
        }

        g.clamp_all();
        g
    }

    /// Sexual offspring: larger jitter for exploratory diversity.
    pub fn mutated(&self, rng: &mut Rng) -> Genome {
        let m = 0.12;
        let mut g = *self;
        g.speed           = g.speed.mutate(rng, m * 0.5);
        g.sense           = g.sense.mutate(rng, m * 3.0);
        g.size            = g.size.mutate(rng, m * 0.4);
        g.metabolism      = g.metabolism.mutate(rng, m * 0.2);
        g.diet            = g.diet.mutate(rng, m * 0.3);
        g.aggression      = g.aggression.mutate(rng, m * 0.3);
        g.repro_threshold = g.repro_threshold.mutate(rng, m * 12.0);
        g.mating_pref     = g.mating_pref.mutate(rng, m * 0.3);
        g.temp_optimum    = g.temp_optimum.mutate(rng, m * 0.3);
        g.temp_tolerance  = g.temp_tolerance.mutate(rng, m * 0.2);
        g.poison_resist   = g.poison_resist.mutate(rng, m * 0.2);
        g.lethality       = g.lethality.mutate(rng, m * 0.3);
        g.feed_efficiency = g.feed_efficiency.mutate(rng, m * 0.2);
        g.swim_capability = g.swim_capability.mutate(rng, m * 0.2);
        g.climb_capability= g.climb_capability.mutate(rng, m * 0.2);
        g.vision_size     = g.vision_size.mutate(rng, m * 0.5);
        g.sociability     = g.sociability.mutate(rng, m * 0.3);
        g.altruism        = g.altruism.mutate(rng, m * 0.3);
        g.graze           = g.graze.mutate(rng, m * 0.3);
        g.claws           = g.claws.mutate(rng, m * 0.3);
        g.defense_spikes  = g.defense_spikes.mutate(rng, m * 0.3);
        g.repro_complexity= g.repro_complexity.mutate(rng, m * 0.3);
        g.social_capacity = g.social_capacity.mutate(rng, m * 0.3);
        g.immunity        = g.immunity.mutate(rng, m * 0.3);
        g.blood_sucking   = g.blood_sucking.mutate(rng, m * 0.3);

        // Diet transition resistance
        let class_before = DietClass::from_diet(self.effective_diet(150.0));
        let class_after  = DietClass::from_diet(g.effective_diet(150.0));
        if class_before != class_after && rng.chance(0.85) {
            g.diet = self.diet;
        }

        g.clamp_all();
        g
    }

    // ─── crossover ───────────────────────────────────────────────────────────

    /// Recombine two parents (sexual reproduction), then mutate.
    pub fn crossover(a: &Genome, b: &Genome, rng: &mut Rng) -> Genome {
        let m = 0.12;
        let diet_gene = {
            let class_a = DietClass::from_diet(a.effective_diet(150.0));
            let class_b = DietClass::from_diet(b.effective_diet(150.0));
            if class_a != class_b {
                if rng.chance(0.5) { a.diet } else { b.diet }
            } else {
                Gene::crossover(&a.diet, &b.diet, rng, m * 0.3)
            }
        };

        let mut g = Genome {
            speed:           Gene::crossover(&a.speed,           &b.speed,           rng, m * 0.5),
            sense:           Gene::crossover(&a.sense,           &b.sense,           rng, m * 3.0),
            size:            Gene::crossover(&a.size,            &b.size,            rng, m * 0.4),
            metabolism:      Gene::crossover(&a.metabolism,      &b.metabolism,      rng, m * 0.2),
            diet:            diet_gene,
            aggression:      Gene::crossover(&a.aggression,      &b.aggression,      rng, m * 0.3),
            repro_threshold: Gene::crossover(&a.repro_threshold, &b.repro_threshold, rng, m * 12.0),
            mating_pref:     Gene::crossover(&a.mating_pref,     &b.mating_pref,     rng, m * 0.3),
            temp_optimum:    Gene::crossover(&a.temp_optimum,    &b.temp_optimum,    rng, m * 0.3),
            temp_tolerance:  Gene::crossover(&a.temp_tolerance,  &b.temp_tolerance,  rng, m * 0.2),
            poison_resist:   Gene::crossover(&a.poison_resist,   &b.poison_resist,   rng, m * 0.2),
            lethality:       Gene::crossover(&a.lethality,       &b.lethality,       rng, m * 0.3),
            feed_efficiency: Gene::crossover(&a.feed_efficiency, &b.feed_efficiency, rng, m * 0.2),
            swim_capability: Gene::crossover(&a.swim_capability, &b.swim_capability, rng, m * 0.2),
            climb_capability:Gene::crossover(&a.climb_capability,&b.climb_capability,rng, m * 0.2),
            vision_size:     Gene::crossover(&a.vision_size,     &b.vision_size,     rng, m * 0.5),
            sociability:     Gene::crossover(&a.sociability,     &b.sociability,     rng, m * 0.3),
            altruism:        Gene::crossover(&a.altruism,        &b.altruism,        rng, m * 0.3),
            graze:           Gene::crossover(&a.graze,           &b.graze,           rng, m * 0.3),
            claws:           Gene::crossover(&a.claws,           &b.claws,           rng, m * 0.3),
            defense_spikes:  Gene::crossover(&a.defense_spikes,  &b.defense_spikes,  rng, m * 0.3),
            repro_complexity:Gene::crossover(&a.repro_complexity,&b.repro_complexity,rng, m * 0.3),
            social_capacity: Gene::crossover(&a.social_capacity, &b.social_capacity, rng, m * 0.3),
            immunity:        Gene::crossover(&a.immunity,        &b.immunity,        rng, m * 0.3),
            blood_sucking:   Gene::crossover(&a.blood_sucking,   &b.blood_sucking,   rng, m * 0.3),
        };
        g.clamp_all();
        g
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral_energy() -> f64 {
        150.0 // matches default repro_threshold midpoint
    }

    #[test]
    fn gene_constant_evaluate() {
        let g = Gene::constant(0.65);
        assert!((g.evaluate(0.0) - 0.65).abs() < 1e-9);
        // Non-zero x should still return 0.65 when b=c=0.
        assert!((g.evaluate(1.0) - 0.65).abs() < 1e-9);
    }

    #[test]
    fn gene_polynomial_evaluate() {
        // f(x) = 1 + 2x + 3x² → f(2) = 1 + 4 + 12 = 17
        let g = Gene { a: 1.0, b: 2.0, c: 3.0 };
        assert!((g.evaluate(2.0) - 17.0).abs() < 1e-9);
    }

    #[test]
    fn efficiency_formulas_match_spec() {
        let mut g = Genome::random(&mut Rng::new(1));
        g.diet = Gene::constant(0.0);
        assert!((g.plant_efficiency(neutral_energy()) - 1.0).abs() < 1e-9);
        assert!(g.meat_efficiency(neutral_energy()).abs() < 1e-9);
        g.diet = Gene::constant(1.0);
        assert!(g.plant_efficiency(neutral_energy()).abs() < 1e-9);
        assert!((g.meat_efficiency(neutral_energy()) - 1.0).abs() < 1e-9);
        g.diet = Gene::constant(0.5);
        assert!((g.plant_efficiency(neutral_energy()) - 0.5).abs() < 1e-9);
        assert!((g.meat_efficiency(neutral_energy()) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn diet_class_bands() {
        let mut rng = Rng::new(1);
        let mut g = Genome::random(&mut rng);
        let e = neutral_energy();
        g.diet = Gene::constant(0.1);
        assert_eq!(g.diet_class_at(e), DietClass::Herbivore);
        g.diet = Gene::constant(0.5);
        assert_eq!(g.diet_class_at(e), DietClass::Omnivore);
        g.diet = Gene::constant(0.9);
        assert_eq!(g.diet_class_at(e), DietClass::Carnivore);
    }

    #[test]
    fn mutation_keeps_baselines_in_range() {
        let mut rng = Rng::new(9);
        let mut g = Genome::random(&mut rng);
        for _ in 0..500 {
            g = g.mutated(&mut rng);
            let e = neutral_energy();
            assert!(g.effective_diet(e) >= 0.0 && g.effective_diet(e) <= 1.0);
            assert!(g.mating_pref.a >= 0.0 && g.mating_pref.a <= 1.0);
            assert!(g.size.a >= 0.4 && g.size.a <= 3.0);
            assert!(g.repro_threshold.a >= 85.0 && g.repro_threshold.a <= 200.0);
            assert!(g.effective_swim(e) >= 0.0 && g.effective_swim(e) <= 1.0);
            assert!(g.effective_climb(e) >= 0.0 && g.effective_climb(e) <= 1.0);
        }
    }

    #[test]
    fn crossover_stays_valid() {
        let mut rng = Rng::new(3);
        let a = Genome::random(&mut rng);
        let b = Genome::random(&mut rng);
        let child = Genome::crossover(&a, &b, &mut rng);
        let e = neutral_energy();
        assert!(child.effective_diet(e) >= 0.0 && child.effective_diet(e) <= 1.0);
        assert!(child.effective_swim(e) >= 0.0 && child.effective_swim(e) <= 1.0);
        assert!(child.effective_climb(e) >= 0.0 && child.effective_climb(e) <= 1.0);
    }

    #[test]
    fn plasticity_shifts_diet_with_energy() {
        // A genome with a strong positive b coefficient on diet should be more
        // carnivorous when well-fed and more herbivorous when starving.
        let mut g = Genome::random(&mut Rng::new(42));
        g.repro_threshold = Gene::constant(150.0);
        g.diet = Gene { a: 0.5, b: 0.4, c: 0.0 }; // strong linear plasticity
        let hungry_diet = g.effective_diet(50.0);  // x ≈ -0.67
        let fed_diet    = g.effective_diet(250.0); // x ≈  0.67
        assert!(fed_diet > hungry_diet, "positive b should increase diet when well-fed");
    }

    #[test]
    fn energy_x_is_zero_at_threshold() {
        let mut g = Genome::random(&mut Rng::new(5));
        g.repro_threshold = Gene::constant(150.0);
        let x = g.energy_x(150.0);
        assert!(x.abs() < 1e-9, "energy_x should be 0 at repro threshold");
    }

    #[test]
    fn new_genes_clamped_and_mutated() {
        let mut rng = Rng::new(10);
        let mut g = Genome::random(&mut rng);
        assert!(g.sociability.a >= -1.0 && g.sociability.a <= 1.0);
        assert!(g.altruism.a >= -1.0 && g.altruism.a <= 1.0);
        assert!(g.graze.a >= 0.0 && g.graze.a <= 1.0);

        for _ in 0..100 {
            g = g.mutated(&mut rng);
            assert!(g.sociability.a >= -1.0 && g.sociability.a <= 1.0);
            assert!(g.altruism.a >= -1.0 && g.altruism.a <= 1.0);
            assert!(g.graze.a >= 0.0 && g.graze.a <= 1.0);
        }
    }

    #[test]
    fn diet_transition_resistance_mutation() {
        let mut rng = Rng::new(42);
        let mut base = Genome::random(&mut rng);
        base.diet = Gene::constant(0.29); 

        let mut changes = 0;
        for _ in 0..1000 {
            let child = base.mutated(&mut rng);
            let class_before = DietClass::from_diet(base.effective_diet(150.0));
            let class_after  = DietClass::from_diet(child.effective_diet(150.0));
            if class_before != class_after {
                changes += 1;
            }
        }
        assert!(changes < 50, "Expected very few class changes due to 85% resistance, but got {}", changes);
    }

    #[test]
    fn diet_transition_resistance_crossover() {
        let mut rng = Rng::new(100);
        let mut a = Genome::random(&mut rng);
        a.diet = Gene::constant(0.1);
        let mut b = Genome::random(&mut rng);
        b.diet = Gene::constant(0.9);

        for _ in 0..100 {
            let child = Genome::crossover(&a, &b, &mut rng);
            let diet_val = child.diet.a;
            assert!(
                (diet_val - 0.1).abs() < 1e-9 || (diet_val - 0.9).abs() < 1e-9,
                "Child diet should be exactly 0.1 or 0.9, but got {}", diet_val
            );
        }
    }
}
