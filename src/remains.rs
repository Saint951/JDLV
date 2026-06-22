//! What the dead leave behind: carcasses and fertilizer.
//!
//! When a creature dies it leaves a **carcass** — meat that carnivores and
//! scavenging decomposers can eat. Carcasses rot over cycles; whatever is left
//! breaks down into **fertilizer** (also produced by living creatures' droppings)
//! which enriches the soil and helps fallen fruit grow into new trees.

use crate::geometry::Vec2;

#[derive(Clone, Debug)]
pub struct Carcass {
    pub pos: Vec2,
    /// Remaining edible energy.
    pub energy: f64,
    /// Cycles this carcass has been rotting (drives decay -> fertilizer).
    pub age_cycles: u32,
}

impl Carcass {
    pub fn new(pos: Vec2, energy: f64) -> Self {
        Carcass {
            pos,
            energy,
            age_cycles: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Fertilizer {
    pub pos: Vec2,
    /// How much nutrient remains; decays each cycle.
    pub amount: f64,
}

impl Fertilizer {
    pub fn new(pos: Vec2, amount: f64) -> Self {
        Fertilizer { pos, amount }
    }
}
