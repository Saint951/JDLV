//! Per-creature episodic memory: a fixed-size circular buffer of recent
//! sensory events.
//!
//! Each creature remembers the last [`MEMORY_SIZE`] things it perceived —
//! food positions, threats, mates, carcasses. When live sensing finds nothing
//! within the creature's sense radius, it navigates toward its freshest
//! matching memory slot instead of wandering randomly.
//!
//! Memory slots age every tick and are evicted after [`MEMORY_TTL`] ticks,
//! so the creature's spatial knowledge degrades naturally when the world
//! moves on.

use crate::geometry::Vec2;

/// Number of memory slots per creature. 32 slots ≈ 2 KB; raise freely given
/// 30 GB RAM (e.g. 4096 slots ≈ 168 MB across 1 500 creatures).
pub const MEMORY_SIZE: usize = 32;

/// Ticks before a memory slot is considered stale and evicted.
pub const MEMORY_TTL: u16 = 120;

/// The kind of thing remembered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemKind {
    Empty,
    Food,
    Carcass,
    Threat,
    Mate,
}

/// A single episodic memory entry.
#[derive(Clone, Copy, Debug)]
pub struct MemSlot {
    pub kind: MemKind,
    /// Remembered world-space position.
    pub pos: Vec2,
    /// Value attached to this memory (energy seen, threat power, etc.).
    pub value: f32,
    /// How many ticks ago this was recorded. Freshest = 0; evicted at TTL.
    pub age: u16,
}

impl Default for MemSlot {
    fn default() -> Self {
        MemSlot {
            kind: MemKind::Empty,
            pos: Vec2::zero(),
            value: 0.0,
            age: u16::MAX,
        }
    }
}

/// Fixed-size circular memory buffer for one creature.
#[derive(Clone, Debug)]
pub struct MemoryBuffer {
    slots: [MemSlot; MEMORY_SIZE],
    write_head: usize,
}

impl MemoryBuffer {
    pub fn new() -> Self {
        MemoryBuffer {
            slots: [MemSlot::default(); MEMORY_SIZE],
            write_head: 0,
        }
    }

    /// Record a fresh observation, overwriting the oldest slot.
    pub fn record(&mut self, kind: MemKind, pos: Vec2, value: f32) {
        self.slots[self.write_head] = MemSlot { kind, pos, value, age: 0 };
        self.write_head = (self.write_head + 1) % MEMORY_SIZE;
    }

    /// Age every slot by one tick; evict stale entries.
    pub fn tick(&mut self) {
        for slot in &mut self.slots {
            if slot.kind != MemKind::Empty {
                slot.age = slot.age.saturating_add(1);
                if slot.age >= MEMORY_TTL {
                    slot.kind = MemKind::Empty;
                }
            }
        }
    }

    /// Return the average Y-coordinate of all non-empty memory slots of the given kind.
    pub fn average_y(&self, kind: MemKind) -> Option<f64> {
        let mut sum = 0.0;
        let mut count = 0;
        for slot in &self.slots {
            if slot.kind == kind {
                sum += slot.pos.y;
                count += 1;
            }
        }
        if count > 0 { Some(sum / count as f64) } else { None }
    }

    /// The position from the freshest, non-stale slot of `kind`, if any.
    pub fn best(&self, kind: MemKind) -> Option<Vec2> {
        self.slots
            .iter()
            .filter(|s| s.kind == kind)
            .min_by_key(|s| s.age)
            .map(|s| s.pos)
    }

    /// Boost the value of the freshest slot matching `kind` at `pos` (within
    /// `radius`). Used when a creature successfully eats something it was
    /// navigating toward — reinforces the memory.
    pub fn reinforce(&mut self, kind: MemKind, pos: Vec2, radius: f64) {
        let r2 = (radius * radius) as f32;
        if let Some(slot) = self
            .slots
            .iter_mut()
            .filter(|s| s.kind == kind && {
                let dx = s.pos.x - pos.x;
                let dy = s.pos.y - pos.y;
                (dx * dx + dy * dy) as f32 <= r2
            })
            .min_by_key(|s| s.age)
        {
            slot.value = (slot.value * 1.2).min(1000.0);
            slot.age = 0; // refresh the timestamp
        }
    }

    /// Iterate over all valid (non-empty) slots for the given kind.
    pub fn of_kind(&self, kind: MemKind) -> impl Iterator<Item = &MemSlot> {
        self.slots.iter().filter(move |s| s.kind == kind)
    }

    /// Inherit memories from a parent, adding an age penalty.
    pub fn inherit_from(&mut self, parent: &MemoryBuffer, age_penalty: u16) {
        for slot in parent.slots.iter() {
            if slot.kind != MemKind::Empty {
                let new_age = slot.age.saturating_add(age_penalty);
                if new_age < MEMORY_TTL {
                    self.slots[self.write_head] = MemSlot {
                        kind: slot.kind,
                        pos: slot.pos,
                        value: slot.value,
                        age: new_age,
                    };
                    self.write_head = (self.write_head + 1) % MEMORY_SIZE;
                }
            }
        }
    }

    /// Inherit mixed memories from two parents, adding an age penalty.
    pub fn inherit_mixed(&mut self, parent_a: &MemoryBuffer, parent_b: &MemoryBuffer, age_penalty: u16) {
        let mut slots_a: Vec<MemSlot> = parent_a.slots.iter().filter(|s| s.kind != MemKind::Empty).copied().collect();
        let mut slots_b: Vec<MemSlot> = parent_b.slots.iter().filter(|s| s.kind != MemKind::Empty).copied().collect();
        slots_a.sort_by_key(|s| s.age);
        slots_b.sort_by_key(|s| s.age);

        let limit = MEMORY_SIZE / 2;
        let mut mixed = Vec::new();
        for &s in slots_a.iter().take(limit) { mixed.push(s); }
        for &s in slots_b.iter().take(limit) { mixed.push(s); }

        for slot in mixed {
            let new_age = slot.age.saturating_add(age_penalty);
            if new_age < MEMORY_TTL {
                self.slots[self.write_head] = MemSlot {
                    kind: slot.kind,
                    pos: slot.pos,
                    value: slot.value,
                    age: new_age,
                };
                self.write_head = (self.write_head + 1) % MEMORY_SIZE;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_retrieves() {
        let mut mem = MemoryBuffer::new();
        mem.record(MemKind::Food, Vec2::new(10.0, 5.0), 40.0);
        assert!(mem.best(MemKind::Food).is_some());
        assert!(mem.best(MemKind::Threat).is_none());
    }

    #[test]
    fn evicts_after_ttl() {
        let mut mem = MemoryBuffer::new();
        mem.record(MemKind::Food, Vec2::new(1.0, 1.0), 10.0);
        for _ in 0..MEMORY_TTL {
            mem.tick();
        }
        assert!(mem.best(MemKind::Food).is_none());
    }

    #[test]
    fn circular_overwrite_does_not_panic() {
        let mut mem = MemoryBuffer::new();
        for i in 0..(MEMORY_SIZE + 5) {
            mem.record(MemKind::Food, Vec2::new(i as f64, 0.0), 1.0);
        }
        assert!(mem.best(MemKind::Food).is_some());
    }

    #[test]
    fn reinforce_resets_age() {
        let mut mem = MemoryBuffer::new();
        mem.record(MemKind::Food, Vec2::new(5.0, 5.0), 20.0);
        // Age a bit.
        for _ in 0..10 {
            mem.tick();
        }
        mem.reinforce(MemKind::Food, Vec2::new(5.1, 5.0), 2.0);
        // Age should be reset to 0; slot freshly seen.
        let age = mem.slots.iter().find(|s| s.kind == MemKind::Food).unwrap().age;
        assert_eq!(age, 0);
    }

    #[test]
    fn test_genomic_memory() {
        let mut parent_a = MemoryBuffer::new();
        parent_a.record(MemKind::Food, Vec2::new(1.0, 2.0), 10.0);
        let mut parent_b = MemoryBuffer::new();
        parent_b.record(MemKind::Threat, Vec2::new(3.0, 4.0), 50.0);

        let mut child = MemoryBuffer::new();
        child.inherit_mixed(&parent_a, &parent_b, 40);

        assert!(child.best(MemKind::Food).is_some());
        assert!(child.best(MemKind::Threat).is_some());
        
        // Ensure they have the age penalty applied
        let slot = child.slots.iter().find(|s| s.kind == MemKind::Food).unwrap();
        assert_eq!(slot.age, 40);
    }
}
