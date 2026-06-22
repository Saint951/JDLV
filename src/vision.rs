//! Local perceptual field (grid vision).
//!
//! Each creature's sense radius is supplemented by a **V × V grid** centred
//! on its position (V = [`VISION_SIZE`], always odd). Every cell summarises
//! the terrain type, food energy, threat level, social affinity, carcass
//! energy, and temperature deviation present in that patch of world.
//!
//! The grid is built fresh each tick from the live world state and is used
//! to compute a weighted movement direction — richer than "nearest food"
//! because the creature weighs *all* sensory channels simultaneously.

use crate::geometry::Vec2;

/// Half-extent of the vision grid; 3 → 7 × 7 cells.
pub const VISION_HALF: usize = 3;

/// Total cells per side (always odd so the creature is at centre).
pub const VISION_SIZE: usize = 2 * VISION_HALF + 1; // 7

/// Per-cell perceptual summary.
#[derive(Clone, Copy, Debug, Default)]
pub struct VisionCell {
    /// Terrain type index: 0=DeepWater 1=ShallowWater 2=Sand 3=Plains 4=Mountain.
    pub terrain: u8,
    /// Total fruit energy visible in this cell.
    pub food_energy: f32,
    /// Total combat power of hostile creatures in this cell (threat if >self).
    pub threat_power: f32,
    /// Mean social-vector cosine similarity toward creatures in this cell.
    pub social_affinity: f32,
    /// Total carcass energy in this cell.
    pub carcass_energy: f32,
    /// Absolute temperature deviation from this creature's optimum.
    pub temp_deviation: f32,
}

/// A snapshot of the creature's local perceptual field for one tick.
pub struct VisionGrid {
    pub cells: [[VisionCell; VISION_SIZE]; VISION_SIZE],
    /// World-space width/height of one vision cell.
    pub cell_size: f64,
    /// World-space centre of the grid (the creature's position).
    pub origin: Vec2,
}

impl VisionGrid {
    /// Create an empty grid centred at `origin` with each cell spanning
    /// `cell_size` world units.
    pub fn new(origin: Vec2, cell_size: f64) -> Self {
        VisionGrid {
            cells: [[VisionCell::default(); VISION_SIZE]; VISION_SIZE],
            cell_size,
            origin,
        }
    }

    /// Convert a world position to grid indices `(col, row)`, if within range.
    pub fn world_to_cell(&self, p: Vec2) -> Option<(usize, usize)> {
        let half = VISION_HALF as f64;
        let dx = (p.x - self.origin.x) / self.cell_size + half;
        let dy = (p.y - self.origin.y) / self.cell_size + half;
        if dx < 0.0 || dy < 0.0 {
            return None;
        }
        let cx = dx as usize;
        let cy = dy as usize;
        if cx < VISION_SIZE && cy < VISION_SIZE {
            Some((cx, cy))
        } else {
            None
        }
    }

    /// The total world-space radius covered by this grid.
    pub fn radius(&self) -> f64 {
        self.cell_size * (VISION_HALF as f64 + 0.5)
    }

    // --- movement hints -------------------------------------------------------

    /// Weighted centroid direction toward the best combined food + carcass signal.
    /// Returns `None` when all cells are empty.
    pub fn best_food_direction(&self) -> Option<Vec2> {
        self.weighted_centroid(|c| (c.food_energy + c.carcass_energy).max(0.0))
    }

    /// Direction away from the highest local threat.
    /// Computes the centroid of the complement: empty cells (no threat) get high
    /// weight so the creature is pulled toward the safest open area.
    pub fn flee_direction(&self) -> Option<Vec2> {
        let max_threat = self
            .cells
            .iter()
            .flat_map(|r| r.iter())
            .map(|c| c.threat_power)
            .fold(0.0_f32, f32::max);
        if max_threat < 1e-6 {
            return None;
        }
        // Weight = max_threat − cell_threat: cells with zero threat get max weight,
        // cells with max threat get weight 0 (excluded).
        self.weighted_centroid(|c| max_threat - c.threat_power)
    }

    /// Direction toward highest social affinity (flocking / kin clustering).
    pub fn social_direction(&self) -> Option<Vec2> {
        self.weighted_centroid(|c| c.social_affinity.max(0.0))
    }

    /// Direction toward the most thermally comfortable cell.
    pub fn comfort_direction(&self) -> Option<Vec2> {
        // prefer cells with *lower* temperature deviation
        let max_dev = self
            .cells
            .iter()
            .flat_map(|row| row.iter())
            .map(|c| c.temp_deviation)
            .fold(0.0_f32, f32::max);
        self.weighted_centroid(|c| (max_dev - c.temp_deviation).max(0.0))
    }

    fn weighted_centroid<F: Fn(&VisionCell) -> f32>(&self, weight: F) -> Option<Vec2> {
        let half = VISION_HALF as f64;
        let mut total_w = 0.0_f64;
        let mut sum = Vec2::zero();
        for gy in 0..VISION_SIZE {
            for gx in 0..VISION_SIZE {
                let w = weight(&self.cells[gy][gx]) as f64;
                if w <= 1e-9 {
                    continue;
                }
                // Offset from grid centre in world units.
                let dx = (gx as f64 - half) * self.cell_size;
                let dy = (gy as f64 - half) * self.cell_size;
                sum = Vec2::new(sum.x + dx * w, sum.y + dy * w);
                total_w += w;
            }
        }
        if total_w > 1e-9 {
            Some(Vec2::new(sum.x / total_w, sum.y / total_w))
        } else {
            None
        }
    }

    /// Best combined movement hint: food first, flee from threats, then comfort.
    /// Returns a unit-scale direction vector (unnormalised — caller normalises).
    /// Blends in flocking (attract/repel based on sociability) when targeting food.
    pub fn movement_hint(&self, sociability: f64) -> Vec2 {
        // flee_direction() already returns None when there's no meaningful threat.
        // When it does return a direction, threat is significant — use it.
        if let Some(flee) = self.flee_direction() {
            return flee;
        }

        // If we have food, blend it with social attraction or repulsion.
        if let Some(food) = self.best_food_direction() {
            if let Some(social) = self.social_direction() {
                if sociability > 0.0 {
                    // Attract to similar neighbours (flocking)
                    let social_w = sociability.clamp(0.0, 1.0) * 0.5;
                    return food.scale(1.0 - social_w).add(social.scale(social_w));
                } else if sociability < 0.0 {
                    // Avoid similar neighbours (solitude)
                    let social_w = (-sociability).clamp(0.0, 1.0) * 0.3;
                    return food.scale(1.0).add(social.scale(-social_w));
                }
            }
            return food;
        }

        self.comfort_direction().unwrap_or_else(Vec2::zero)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_to_cell_centre() {
        let g = VisionGrid::new(Vec2::new(40.0, 20.0), 2.0);
        let (cx, cy) = g.world_to_cell(Vec2::new(40.0, 20.0)).unwrap();
        assert_eq!((cx, cy), (VISION_HALF, VISION_HALF));
    }

    #[test]
    fn world_to_cell_far_outside_is_none() {
        let g = VisionGrid::new(Vec2::new(40.0, 20.0), 2.0);
        assert!(g.world_to_cell(Vec2::new(100.0, 100.0)).is_none());
    }

    #[test]
    fn food_direction_points_toward_food() {
        let mut g = VisionGrid::new(Vec2::new(0.0, 0.0), 1.0);
        // Place food in the right column (gx = VISION_SIZE-1).
        for gy in 0..VISION_SIZE {
            g.cells[gy][VISION_SIZE - 1].food_energy = 100.0;
        }
        let dir = g.best_food_direction().unwrap();
        assert!(dir.x > 0.0, "should point right toward food");
    }

    #[test]
    fn flee_direction_points_away_from_threat() {
        let mut g = VisionGrid::new(Vec2::new(0.0, 0.0), 1.0);
        // Threat on the left.
        for gy in 0..VISION_SIZE {
            g.cells[gy][0].threat_power = 10.0;
        }
        let dir = g.flee_direction().unwrap();
        assert!(dir.x > 0.0, "should flee rightward away from left threat");
    }
}
