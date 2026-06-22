//! Procedural terrain: Perlin noise → discrete tile types → per-tile movement
//! and temperature modifiers.
//!
//! The terrain map is generated once at world creation from the RNG seed and
//! never changes during a run, giving each seed a unique, reproducible landscape.

use crate::genome::Genome;
use crate::rng::Rng;

// ---------------------------------------------------------------------------
// Perlin noise (Ken Perlin's 2002 "improved" algorithm, 2-D version)
// ---------------------------------------------------------------------------

struct Perlin {
    /// Doubled permutation table — avoids a modulo on the second lookup.
    perm: [usize; 512],
}

impl Perlin {
    fn new(rng: &mut Rng) -> Self {
        let mut p = [0usize; 256];
        for i in 0..256 {
            p[i] = i;
        }
        // Fisher-Yates in-place shuffle using the deterministic RNG.
        for i in (1..256).rev() {
            let j = rng.below(i + 1);
            p.swap(i, j);
        }
        let mut perm = [0usize; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Perlin { perm }
    }

    fn fade(t: f64) -> f64 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn lerp(a: f64, b: f64, t: f64) -> f64 {
        a + t * (b - a)
    }

    fn grad(h: usize, x: f64, y: f64) -> f64 {
        match h & 3 {
            0 =>  x + y,
            1 => -x + y,
            2 =>  x - y,
            _ => -x - y,
        }
    }

    /// Single-octave Perlin noise in approximately [-1, 1].
    /// Input coordinates must be non-negative.
    fn sample(&self, x: f64, y: f64) -> f64 {
        let xi = x.floor() as usize & 255;
        let yi = y.floor() as usize & 255;
        let xf = x - x.floor();
        let yf = y - y.floor();
        let u = Self::fade(xf);
        let v = Self::fade(yf);

        let aa = self.perm[self.perm[xi    ] + yi    ];
        let ab = self.perm[self.perm[xi    ] + yi + 1];
        let ba = self.perm[self.perm[xi + 1] + yi    ];
        let bb = self.perm[self.perm[xi + 1] + yi + 1];

        Self::lerp(
            Self::lerp(Self::grad(aa, xf,       yf      ),
                       Self::grad(ba, xf - 1.0, yf      ), u),
            Self::lerp(Self::grad(ab, xf,       yf - 1.0),
                       Self::grad(bb, xf - 1.0, yf - 1.0), u),
            v,
        )
    }

    /// `n`-octave fractal Brownian motion with persistence 0.5, result in [-1, 1].
    fn octaves(&self, x: f64, y: f64, n: u32) -> f64 {
        let mut val = 0.0_f64;
        let mut amp = 1.0_f64;
        let mut total_amp = 0.0_f64;
        for i in 0..n {
            let f = (1u32 << i) as f64;
            val += self.sample(x * f, y * f) * amp;
            total_amp += amp;
            amp *= 0.5;
        }
        val / total_amp
    }
}

// ---------------------------------------------------------------------------
// Tile types
// ---------------------------------------------------------------------------

/// The terrain type at a world cell. Determines movement costs, temperature
/// offsets, and where trees may germinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileType {
    /// Impassable to low-swim creatures; no tree spawns.
    DeepWater,
    /// Slow and draining to cross; acts as a scavenging/water hub.
    ShallowWater,
    /// Sandy/coastal; reduced fertility and slight movement drag.
    Sand,
    /// Optimal habitat: standard energy, high fertility.
    Plains,
    /// High altitude: doubled movement cost, permanently cold.
    Mountain,
}

impl TileType {
    fn from_noise(v: f64) -> Self {
        if      v < 0.28 { TileType::DeepWater    }
        else if v < 0.42 { TileType::ShallowWater }
        else if v < 0.48 { TileType::Sand         }
        else if v < 0.85 { TileType::Plains       }
        else             { TileType::Mountain      }
    }

    /// ASCII character used as the tile background in the rendered map.
    pub fn render_char(self) -> char {
        match self {
            TileType::DeepWater    => '=',
            TileType::ShallowWater => '~',
            TileType::Sand         => ',',
            TileType::Plains       => ' ',
            TileType::Mountain     => '^',
        }
    }

    /// Extra energy cost per unit of distance moved on this tile.
    /// Terrain-adaption genes (`swim_capability`, `climb_capability`) reduce it.
    /// `energy` is passed so polynomial genes are evaluated at the current state.
    pub fn movement_penalty(self, genome: &Genome, energy: f64) -> f64 {
        let swim  = genome.effective_swim(energy);
        let climb = genome.effective_climb(energy);
        match self {
            TileType::DeepWater => {
                // Very expensive; swim_capability significantly reduces this.
                4.0 * (1.0 - swim * 0.8).max(0.1)
            }
            TileType::ShallowWater => {
                0.6 * (1.0 - swim * 0.7).max(0.05)
            }
            TileType::Sand    => 0.15,
            TileType::Plains  => 0.0,
            TileType::Mountain => {
                // ≈doubled movement cost; climb_capability reduces toward zero.
                1.2 * (1.0 - climb * 0.75).max(0.08)
            }
        }
    }

    /// Ambient temperature shift for creatures standing on this tile.
    /// Mountains are permanently colder; `climb_capability` halves the penalty.
    /// `energy` is passed so the polynomial climb gene is evaluated correctly.
    pub fn temperature_offset(self, genome: &Genome, mountain_cold: f64, energy: f64) -> f64 {
        if self == TileType::Mountain {
            let climb = genome.effective_climb(energy);
            -mountain_cold * (1.0 - climb * 0.5)
        } else {
            0.0
        }
    }

    /// A creature with insufficient swim capability cannot enter deep water.
    /// `energy` is passed so the polynomial swim gene is evaluated correctly.
    pub fn is_accessible(self, genome: &Genome, energy: f64) -> bool {
        self != TileType::DeepWater || genome.effective_swim(energy) >= 0.25
    }

    /// Trees may only be placed on dry land.
    pub fn allows_trees(self) -> bool {
        !matches!(self, TileType::DeepWater | TileType::ShallowWater)
    }

    /// Fruit-drop rate multiplier for trees rooted on this tile.
    pub fn tree_fertility_factor(self) -> f64 {
        match self {
            TileType::DeepWater    => 0.0,
            TileType::ShallowWater => 0.0,
            TileType::Sand         => 0.3,
            TileType::Plains       => 1.0,
            TileType::Mountain     => 0.5,
        }
    }
}

// ---------------------------------------------------------------------------
// Terrain map
// ---------------------------------------------------------------------------

/// Precomputed grid of tile types covering the whole world.
pub struct TerrainMap {
    pub(crate) tiles: Vec<TileType>,
    width: usize,
    height: usize,
}

impl TerrainMap {
    /// Generate terrain for a world of the given floating-point dimensions.
    /// Uses a noise seed derived from `world_seed` (independent of the
    /// creature/ecology RNG so terrain doesn't affect reproducibility of
    /// biological events, only the landscape itself).
    pub fn generate(world_w: f64, world_h: f64, world_seed: u64) -> Self {
        let width  = world_w.ceil() as usize + 1;
        let height = world_h.ceil() as usize + 1;

        // Separate RNG so terrain generation doesn't consume creature-RNG steps.
        let mut rng = Rng::new(world_seed ^ 0x6c62272e07bb0142);
        let noise = Perlin::new(&mut rng);

        // Scale so ~4 landscape periods fit the world's shorter axis.
        let scale = 4.0 / world_w.min(world_h);
        let mut tiles = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let raw = noise.octaves(x as f64 * scale, y as f64 * scale, 4);
                // Remap [-1, 1] → [0, 1] and classify.
                let v = ((raw + 1.0) * 0.5).clamp(0.0, 1.0);
                tiles.push(TileType::from_noise(v));
            }
        }
        TerrainMap { tiles, width, height }
    }

    /// Tile at a world position (clamped to map bounds).
    pub fn tile_at(&self, x: f64, y: f64) -> TileType {
        let xi = (x as usize).min(self.width  - 1);
        let yi = (y as usize).min(self.height - 1);
        self.tiles[yi * self.width + xi]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_map_covers_full_world() {
        let map = TerrainMap::generate(80.0, 40.0, 1);
        // Corner lookups should not panic.
        let _ = map.tile_at(0.0, 0.0);
        let _ = map.tile_at(79.9, 39.9);
        let _ = map.tile_at(80.0, 40.0); // clamped to edge
    }

    #[test]
    fn tile_types_cover_full_noise_range() {
        assert_eq!(TileType::from_noise(0.0),   TileType::DeepWater);
        assert_eq!(TileType::from_noise(0.35),  TileType::ShallowWater);
        assert_eq!(TileType::from_noise(0.45),  TileType::Sand);
        assert_eq!(TileType::from_noise(0.60),  TileType::Plains);
        assert_eq!(TileType::from_noise(0.90),  TileType::Mountain);
        assert_eq!(TileType::from_noise(1.0),   TileType::Mountain);
    }

    #[test]
    fn deep_water_blocks_low_swim_creatures() {
        use crate::rng::Rng;
        use crate::genome::Gene;
        let mut rng = Rng::new(1);
        let mut g = crate::genome::Genome::random(&mut rng);
        g.swim_capability = Gene::constant(0.1);
        assert!(!TileType::DeepWater.is_accessible(&g, 150.0));
        g.swim_capability = Gene::constant(0.5);
        assert!(TileType::DeepWater.is_accessible(&g, 150.0));
    }

    #[test]
    fn mountain_is_colder_and_reducible_by_climb() {
        use crate::rng::Rng;
        use crate::genome::Gene;
        let mut rng = Rng::new(1);
        let mut g = crate::genome::Genome::random(&mut rng);
        g.climb_capability = Gene::constant(0.0);
        let cold_full = TileType::Mountain.temperature_offset(&g, 0.3, 150.0);
        g.climb_capability = Gene::constant(1.0);
        let cold_climb = TileType::Mountain.temperature_offset(&g, 0.3, 150.0);
        assert!(cold_full < 0.0);           // mountains are colder
        assert!(cold_climb > cold_full);    // climb reduces the penalty
    }

    #[test]
    fn different_seeds_produce_different_terrain() {
        let a = TerrainMap::generate(80.0, 40.0, 1);
        let b = TerrainMap::generate(80.0, 40.0, 2);
        // At least some tiles must differ.
        let same = a.tiles.iter().zip(&b.tiles).filter(|(x, y)| x == y).count();
        assert!(same < a.tiles.len());
    }
}
