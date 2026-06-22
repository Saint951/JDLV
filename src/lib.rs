//! Game of Life — an artificial-life simulation.
//!
//! The classic "Game of Life" is a cellular automaton; this is a different,
//! ecological take on the name. Creatures live on a 2D plane, eat fruit or each
//! other, and must **reproduce enough to survive the end of each cycle**, where
//! every survivor faces an escalating death roll (30% + 5% per cycle survived).
//!
//! Modules:
//! - [`config`]   — tunable run parameters
//! - [`rng`]      — deterministic RNG (seeded, dependency-free)
//! - [`geometry`] — 2D vector math
//! - [`genome`]   — heritable traits as polynomial genes, mutation, crossover
//! - [`creature`] — the agents (with memory + social embedding)
//! - [`memory`]   — per-creature episodic memory buffer (Phase 1)
//! - [`vision`]   — per-creature local perceptual grid (Phase 2)
//! - [`social`]   — social embedding vectors for kin recognition (Phase 3)
//! - [`food`]     — food sources and fruit
//! - [`world`]    — the simulation engine
//! - [`render`]   — ASCII visualisation
//! - [`server`]   — embedded HTTP server for the web viewer (Phase 5)

pub mod climate;
pub mod config;
pub mod creature;
pub mod food;
pub mod genome;
pub mod geometry;
pub mod gut;
pub mod memory;
pub mod remains;
pub mod render;
pub mod rng;
pub mod server;
pub mod social;
pub mod terrain;
pub mod vision;
pub mod world;

pub use config::Config;
pub use world::World;
