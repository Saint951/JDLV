//! ASCII rendering: the world map, a stats footer, and the population graph.

use crate::world::{Stats, World};

/// Render the world map (grid + a stats footer) into a `String`.
pub fn frame(world: &World) -> String {
    let cols = world.cfg.render_cols.max(1);
    let rows = world.cfg.render_rows.max(1);
    let mut grid = vec![vec![' '; cols]; rows];

    let sx = cols as f64 / world.cfg.width;
    let sy = rows as f64 / world.cfg.height;
    let to_cell = |x: f64, y: f64| -> (usize, usize) {
        let cx = ((x * sx) as usize).min(cols - 1);
        let cy = ((y * sy) as usize).min(rows - 1);
        (cx, cy)
    };

    // Terrain background (lowest layer — entities render on top).
    for cy in 0..rows {
        for cx in 0..cols {
            let wx = (cx as f64 + 0.5) * world.cfg.width / cols as f64;
            let wy = (cy as f64 + 0.5) * world.cfg.height / rows as f64;
            grid[cy][cx] = world.terrain.tile_at(wx, wy).render_char();
        }
    }

    // Helper: true if a cell still shows its terrain background (no entity yet).
    let is_bg = |c: char| matches!(c, '=' | '~' | ',' | ' ' | '^');

    // Aquatic plants (render as '≈' in water tiles, before fruit).
    for ap in &world.aquatic_plants {
        let (cx, cy) = to_cell(ap.pos.x, ap.pos.y);
        if matches!(grid[cy][cx], '=' | '~') {
            grid[cy][cx] = if ap.deep { '=' } else { '~' };
        }
    }
    // Fruit (above terrain).
    for f in &world.fruits {
        let (cx, cy) = to_cell(f.pos.x, f.pos.y);
        if is_bg(grid[cy][cx]) {
            grid[cy][cx] = '.';
        }
    }
    // Carcasses.
    for c in &world.carcasses {
        let (cx, cy) = to_cell(c.pos.x, c.pos.y);
        if is_bg(grid[cy][cx]) || grid[cy][cx] == '.' {
            grid[cy][cx] = '%';
        }
    }
    // Food sources (trees).
    for s in &world.sources {
        let (cx, cy) = to_cell(s.pos.x, s.pos.y);
        grid[cy][cx] = if s.genome.poison > 0.4 { '!' } else { 'T' };
    }
    // Creatures on top.
    for c in &world.creatures {
        if !c.alive {
            continue;
        }
        let (cx, cy) = to_cell(c.pos.x, c.pos.y);
        grid[cy][cx] = c.symbol();
    }

    let mut out = String::new();
    let border: String = "-".repeat(cols + 2);
    out.push_str(&border);
    out.push('\n');
    for row in &grid {
        out.push('|');
        out.extend(row.iter());
        out.push('|');
        out.push('\n');
    }
    out.push_str(&border);
    out.push('\n');
    out.push_str(&stats_line(world));
    out
}

fn stats_line(world: &World) -> String {
    let s: Stats = world.stats();
    format!(
        "cycle {:>3}  tick {:>5}  {:<6}  pop {:>4}  fruit {:>4}  carcass {:>3}  \
         trees {:>3}  aquatic {:>3}  H/O/C {:>3}/{:>3}/{:<3}\n\
         creature avg: energy {:>6.1}  diet {:>4.2}  mating {:>4.2}  \
         temp-opt {:>5.2}  hardiness {:>4.2}  poison-resist {:>4.2}\n\
         carnivore avg: lethality {:>4.2}  feed-eff {:>4.2}   \
         forest avg: poison {:>4.2}  fertility {:>4.2}  seed-disp {:>4.2}  drought-res {:>4.2}\n\
         terrain adapt: swim {:>4.2}  climb {:>4.2}\n\
         legend: H herb  O omni  C carn  (UPPER=asexual, lower=sexual)  \
         T tree  ! poison-tree  . fruit  % carcass\n\
         terrain: = deep-water  ~ shallow  , sand  (space) plains  ^ mountain",
        world.cycle,
        world.tick,
        s.season,
        s.population,
        s.fruits,
        s.carcasses,
        s.trees,
        s.aquatic_plants,
        s.herbivores,
        s.omnivores,
        s.carnivores,
        s.avg_energy,
        s.avg_diet,
        s.avg_mating_pref,
        s.avg_temp_optimum,
        s.avg_temp_tolerance,
        s.avg_poison_resist,
        s.avg_lethality,
        s.avg_feed_efficiency,
        s.avg_tree_poison,
        s.avg_tree_fertility,
        s.avg_seed_dispersal,
        s.avg_drought_resist,
        s.avg_swim_capability,
        s.avg_climb_capability,
    )
}

/// One compact line for headless / per-cycle logging.
pub fn summary_line(world: &World) -> String {
    let s = world.stats();
    format!(
        "c{:>3} {:<6} | pop {:>4} | H {:>3} O {:>3} C {:>3} | trees {:>3} aqua {:>3} carc {:>3} | \
         diet {:>4.2} | t-opt {:>5.2} | leth {:>4.2} | tox {:>4.2} | res {:>4.2} | \
         swim {:>4.2} | disp {:>4.2} | dr {:>4.2}",
        world.cycle,
        s.season,
        s.population,
        s.herbivores,
        s.omnivores,
        s.carnivores,
        s.trees,
        s.aquatic_plants,
        s.carcasses,
        s.avg_diet,
        s.avg_temp_optimum,
        s.avg_lethality,
        s.avg_tree_poison,
        s.avg_poison_resist,
        s.avg_swim_capability,
        s.avg_seed_dispersal,
        s.avg_drought_resist,
    )
}

/// A real-time ASCII line graph of Herbivore / Omnivore / Carnivore counts over
/// cycles — the emergent "Red Queen's Race". Series: `H`, `O`, `C`.
pub fn population_graph(world: &World, height: usize) -> String {
    let h = height.max(5);
    if world.history.is_empty() {
        return String::from("(no cycles recorded yet)");
    }

    // Down-sample columns to fit a sensible width.
    let max_w = world.cfg.render_cols.max(20);
    let n = world.history.len();
    let stride = (n + max_w - 1) / max_w.max(1);
    let stride = stride.max(1);
    let cols: Vec<&crate::world::CycleRecord> =
        world.history.iter().step_by(stride).collect();
    let w = cols.len();

    // Auto-scale the y-axis to the largest total population seen.
    let mut peak = 1usize;
    for r in &cols {
        peak = peak.max(r.herbivores).max(r.omnivores).max(r.carnivores);
    }

    // Build the plot grid. Later series overwrite earlier ones at ties; we draw
    // H, then O, then C so each is visible where it's the local maximum.
    let mut plot = vec![vec![' '; w]; h];
    let row_of = |count: usize| -> usize {
        // Higher counts -> higher rows (row 0 is the top).
        let frac = count as f64 / peak as f64;
        let r = ((1.0 - frac) * (h - 1) as f64).round() as usize;
        r.min(h - 1)
    };
    for (x, r) in cols.iter().enumerate() {
        plot[row_of(r.herbivores)][x] = 'H';
        plot[row_of(r.omnivores)][x] = 'O';
        plot[row_of(r.carnivores)][x] = 'C';
    }

    let mut out = String::new();
    out.push_str("Population over cycles (Red Queen's Race):\n");
    for (ri, row) in plot.iter().enumerate() {
        // Y-axis labels at the top, middle and bottom.
        let label = if ri == 0 {
            format!("{:>4} |", peak)
        } else if ri == h - 1 {
            format!("{:>4} |", 0)
        } else if ri == h / 2 {
            format!("{:>4} |", peak / 2)
        } else {
            "     |".to_string()
        };
        out.push_str(&label);
        out.extend(row.iter());
        out.push('\n');
    }
    // X-axis.
    out.push_str("     +");
    out.push_str(&"-".repeat(w));
    out.push('\n');
    let first = cols.first().map(|r| r.cycle).unwrap_or(0);
    let last = cols.last().map(|r| r.cycle).unwrap_or(0);
    out.push_str(&format!(
        "      cycle {} .. {}   (H=herbivore  O=omnivore  C=carnivore)",
        first, last
    ));
    out
}
