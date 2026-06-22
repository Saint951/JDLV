//! Gut microbiome module: models a creature's internal gut bacterial population.
//!
//! The microbiome dynamically shifts based on what a creature eats (plants/fruits
//! vs meat/carcasses). In turn, it boosts the creature's feeding efficiency for that
//! food type and influences their mood/aggression (mood-microbiome-brain axis).

use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GutBacterium {
    /// 0.0..1.0, boosts plant eating efficiency
    pub plant_fit: f64,
    /// 0.0..1.0, boosts meat eating efficiency
    pub meat_fit: f64,
    /// -0.5..0.5, mood/aggression adjustment
    pub mood_aggression: f64,
}

impl Default for GutBacterium {
    fn default() -> Self {
        GutBacterium {
            plant_fit: 0.5,
            meat_fit: 0.5,
            mood_aggression: 0.0,
        }
    }
}

impl GutBacterium {
    /// Initialize with a random composition.
    pub fn random(rng: &mut Rng) -> Self {
        GutBacterium {
            plant_fit: rng.next_f64(),
            meat_fit: rng.next_f64(),
            mood_aggression: rng.range_f64(-0.5, 0.5),
        }
    }

    /// Return a slightly mutated copy (used during offspring transmission).
    pub fn mutated(&self, rng: &mut Rng) -> Self {
        let jitter = 0.08;
        GutBacterium {
            plant_fit: (self.plant_fit + rng.range_f64(-jitter, jitter)).clamp(0.0, 1.0),
            meat_fit: (self.meat_fit + rng.range_f64(-jitter, jitter)).clamp(0.0, 1.0),
            mood_aggression: (self.mood_aggression + rng.range_f64(-jitter, jitter)).clamp(-0.5, 0.5),
        }
    }

    /// Shift the gut bacterium composition according to digested food.
    ///
    /// Plant/grass consumption shifts fit toward plants and calms the creature's mood.
    /// Meat/prey consumption shifts fit toward meat and incites aggressive behavior.
    pub fn digest_food(&mut self, plant_ratio: f64, meat_ratio: f64) {
        let shift = 0.03;
        if plant_ratio > 0.0 {
            self.plant_fit = (self.plant_fit + shift).min(1.0);
            self.meat_fit = (self.meat_fit - shift * 0.5).max(0.0);
            // Calm mood slightly
            self.mood_aggression = (self.mood_aggression - shift * 0.2).max(-0.5);
        }
        if meat_ratio > 0.0 {
            self.meat_fit = (self.meat_fit + shift).min(1.0);
            self.plant_fit = (self.plant_fit - shift * 0.5).max(0.0);
            // Incite aggression/mood
            self.mood_aggression = (self.mood_aggression + shift * 0.3).min(0.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn test_gut_default() {
        let gut = GutBacterium::default();
        assert_eq!(gut.plant_fit, 0.5);
        assert_eq!(gut.meat_fit, 0.5);
        assert_eq!(gut.mood_aggression, 0.0);
    }

    #[test]
    fn test_gut_random() {
        let mut rng = Rng::new(12345);
        let gut = GutBacterium::random(&mut rng);
        assert!(gut.plant_fit >= 0.0 && gut.plant_fit <= 1.0);
        assert!(gut.meat_fit >= 0.0 && gut.meat_fit <= 1.0);
        assert!(gut.mood_aggression >= -0.5 && gut.mood_aggression <= 0.5);
    }

    #[test]
    fn test_gut_mutation() {
        let mut rng = Rng::new(42);
        let parent = GutBacterium {
            plant_fit: 0.9,
            meat_fit: 0.1,
            mood_aggression: 0.4,
        };
        let child = parent.mutated(&mut rng);
        // child values should be within jitter (0.08) of parent and clamped
        assert!((child.plant_fit - parent.plant_fit).abs() <= 0.08);
        assert!((child.meat_fit - parent.meat_fit).abs() <= 0.08);
        assert!((child.mood_aggression - parent.mood_aggression).abs() <= 0.08);
    }

    #[test]
    fn test_gut_digestion() {
        let mut gut = GutBacterium {
            plant_fit: 0.5,
            meat_fit: 0.5,
            mood_aggression: 0.0,
        };
        // Eating plant should increase plant_fit, decrease meat_fit, calm mood
        gut.digest_food(1.0, 0.0);
        assert!(gut.plant_fit > 0.5);
        assert!(gut.meat_fit < 0.5);
        assert!(gut.mood_aggression < 0.0);

        // Eating meat should increase meat_fit, decrease plant_fit, increase aggression
        let before_meat = gut;
        gut.digest_food(0.0, 1.0);
        assert!(gut.meat_fit > before_meat.meat_fit);
        assert!(gut.plant_fit < before_meat.plant_fit);
        assert!(gut.mood_aggression > before_meat.mood_aggression);
    }
}

