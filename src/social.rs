//! Social embedding vectors for kin recognition, flocking, and mate preference.
//!
//! Each creature carries a compact [`SocialVec`] — a [`SOCIAL_DIM`]-dimensional
//! float vector that summarises its recent social neighbourhood. Every tick the
//! vector drifts toward a running average of nearby neighbours' own vectors,
//! using a slow exponential mix (`α = 0.92`).
//!
//! ### Emergent effects
//! - **Kin recognition** — creatures in the same "social cluster" have high
//!   cosine similarity; predators are less likely to attack kin.
//! - **Flocking** — the vision grid uses social affinity to bias movement
//!   toward similar-vector neighbours, creating loose herds.
//! - **Mate preference** — sexual reproduction preferentially pairs creatures
//!   with similar social vectors, reinforcing genetic coherence within groups.

/// Dimensionality of the social embedding vector.
pub const SOCIAL_DIM: usize = 10;

/// Mixing coefficient for the exponential running average update.
/// Close to 1.0 = slow drift (long social memory).
pub const SOCIAL_ALPHA: f32 = 0.92;

/// A per-creature social embedding vector.
///
/// Initialised from a creature's genome at birth (giving each creature a
/// unique starting identity), then updated by proximity contacts each tick.
#[derive(Clone, Debug)]
pub struct SocialVec(pub [f32; SOCIAL_DIM]);

impl SocialVec {
    /// Initialise from ten heritable genome traits, normalised to [0, 1].
    /// This gives every creature a unique identity at birth while keeping
    /// socially-similar animals in adjacent regions of embedding space.
    #[allow(clippy::too_many_arguments)]
    pub fn from_genome(
        diet: f64,
        size: f64,
        speed: f64,
        mating_pref: f64,
        temp_optimum: f64,
        aggression: f64,
        lethality: f64,
        feed_efficiency: f64,
        sociability: f64,
        altruism: f64,
    ) -> Self {
        SocialVec([
            diet as f32,
            (size as f32 / 3.0).clamp(0.0, 1.0), // size is 0.4..3.0
            (speed as f32 / 4.0).clamp(0.0, 1.0), // speed is 0.2..4.0
            mating_pref as f32,
            ((temp_optimum as f32 + 1.0) / 3.0).clamp(0.0, 1.0), // temp -1..2
            aggression as f32,
            lethality as f32,
            feed_efficiency as f32,
            ((sociability as f32 + 1.0) * 0.5).clamp(0.0, 1.0),
            ((altruism as f32 + 1.0) * 0.5).clamp(0.0, 1.0),
        ])
    }

    /// Cosine similarity in `[-1, 1]`; `1.0` = identical social profile.
    pub fn cosine_similarity(&self, other: &SocialVec) -> f32 {
        let dot: f32 = self.0.iter().zip(&other.0).map(|(a, b)| a * b).sum();
        let mag_a: f32 = self.0.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mag_b: f32 = other.0.iter().map(|x| x * x).sum::<f32>().sqrt();
        if mag_a < 1e-6 || mag_b < 1e-6 {
            0.0
        } else {
            (dot / (mag_a * mag_b)).clamp(-1.0, 1.0)
        }
    }

    /// Drift this vector toward `other` using mixing coefficient `alpha`.
    /// Called every tick for each nearby neighbour, slowly pulling the
    /// creature into its social neighbourhood.
    pub fn mix_toward(&mut self, other: &SocialVec, alpha: f32) {
        for (s, o) in self.0.iter_mut().zip(&other.0) {
            *s = alpha * *s + (1.0 - alpha) * o;
        }
    }

    /// Weighted average of a slice of social vectors (for the group update).
    /// Returns `None` for an empty slice.
    pub fn average(vecs: &[&SocialVec]) -> Option<SocialVec> {
        if vecs.is_empty() {
            return None;
        }
        let mut sum = [0.0f32; SOCIAL_DIM];
        for v in vecs {
            for (s, x) in sum.iter_mut().zip(&v.0) {
                *s += x;
            }
        }
        let n = vecs.len() as f32;
        Some(SocialVec(sum.map(|x| x / n)))
    }
}

impl Default for SocialVec {
    fn default() -> Self {
        SocialVec([0.5; SOCIAL_DIM])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(v: f32) -> SocialVec {
        SocialVec([v; SOCIAL_DIM])
    }

    #[test]
    fn identical_vectors_have_similarity_one() {
        let a = uniform(0.5);
        assert!((a.cosine_similarity(&a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn orthogonal_vectors_have_similarity_zero() {
        let mut a_arr = [0.0f32; SOCIAL_DIM];
        let mut b_arr = [0.0f32; SOCIAL_DIM];
        a_arr[0] = 1.0;
        b_arr[1] = 1.0;
        let a = SocialVec(a_arr);
        let b = SocialVec(b_arr);
        assert!(a.cosine_similarity(&b).abs() < 1e-5);
    }

    #[test]
    fn mix_toward_converges() {
        let mut a = uniform(0.0);
        let b = uniform(1.0);
        for _ in 0..300 {
            a.mix_toward(&b, SOCIAL_ALPHA);
        }
        // After many updates toward [1,…] the vector should be very close to 1.
        assert!(a.0[0] > 0.95, "expected convergence, got {}", a.0[0]);
    }

    #[test]
    fn average_of_two_extremes() {
        let a = uniform(0.0);
        let b = uniform(1.0);
        let avg = SocialVec::average(&[&a, &b]).unwrap();
        assert!((avg.0[0] - 0.5).abs() < 1e-5);
    }
}
