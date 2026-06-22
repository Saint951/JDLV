//! Minimal 2D vector math for positions and movement on the world plane.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    pub fn zero() -> Self {
        Vec2 { x: 0.0, y: 0.0 }
    }

    pub fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }

    pub fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }

    pub fn scale(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }

    pub fn len(self) -> f64 {
        self.dist_sq(Vec2::zero()).sqrt()
    }

    pub fn dist_sq(self, o: Vec2) -> f64 {
        let dx = self.x - o.x;
        let dy = self.y - o.y;
        dx * dx + dy * dy
    }

    pub fn dist(self, o: Vec2) -> f64 {
        self.dist_sq(o).sqrt()
    }

    /// Unit vector in the same direction; returns zero for a zero vector.
    pub fn normalized(self) -> Vec2 {
        let l = self.len();
        if l <= f64::EPSILON {
            Vec2::zero()
        } else {
            self.scale(1.0 / l)
        }
    }

    /// Clamp the position to stay inside `[0, w) x [0, h)`.
    pub fn clamp_to(self, w: f64, h: f64) -> Vec2 {
        Vec2::new(
            self.x.clamp(0.0, (w - 1e-9).max(0.0)),
            self.y.clamp(0.0, (h - 1e-9).max(0.0)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_euclidean() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(3.0, 4.0);
        assert!((a.dist(b) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn normalized_has_unit_length() {
        let v = Vec2::new(3.0, 4.0).normalized();
        assert!((v.len() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn normalized_zero_stays_zero() {
        assert_eq!(Vec2::zero().normalized(), Vec2::zero());
    }

    #[test]
    fn clamp_keeps_inside_bounds() {
        let p = Vec2::new(-5.0, 100.0).clamp_to(10.0, 10.0);
        assert!(p.x >= 0.0 && p.x < 10.0);
        assert!(p.y >= 0.0 && p.y < 10.0);
    }
}
