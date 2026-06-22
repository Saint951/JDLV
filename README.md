# Game of Life (in Rust)

An artificial-life simulation — *not* Conway's cellular automaton, but an
ecological take on the name. Creatures live on a 2D plane, forage and fight for
energy, migrate across a shifting seasonal climate, decompose the dead, and must
**reproduce enough to survive the end of each cycle**. Trees evolve too — growing
poisonous, spreading by seed, and feeding on the soil the dead leave behind.

Written in safe, dependency-free Rust (the RNG, vector math and entire ecosystem
are hand-rolled), so it builds offline with nothing but `cargo`.

## The rules

The simulation implements the design spec directly:

| Spec | Implementation |
| --- | --- |
| **Goal:** reproduce enough to survive at the end of a cycle | A lineage wins by staying alive through repeated end-of-cycle death rolls. |
| **Cycle:** food sources spawn a plausible distance from where they once were | Each cycle every tree drifts (`migrate`) toward the season's favourable latitude, capped at `source_relocate_distance`. |
| **Death:** 30% + 5% per cycle survived (hard cap at 14 cycles) | `death_probability = (0.30 + 0.05 * cycles_survived).min(1.0)`, rolled at every cycle end. Survival gets *harder* each cycle. |
| **Food:** eat other creatures or eat fallen fruit | Herbivores eat fruit; carnivores hunt prey; everything with some carnivory scavenges carcasses. Diet is continuous (0 = herbivore, 1 = carnivore). |
| **Efficiency:** fruit × (1 − diet), meat × diet | Pure specialists are 100%/0% efficient; omnivores (diet ≈ 0.5) pay a 50% penalty on both. |
| **Reproduce — asexual:** 100% cost on the producer | One parent pays the full 100-energy `repro_cost`; offspring is a mutated clone. |
| **Reproduce — sexual:** cost split between participants | Two proximate partners each pay 50; offspring is a genome crossover, then mutated. |
| **Starvation:** ≤ 0 energy at the exact end of a cycle | Checked at cycle end, independent of (and in addition to) the age-death roll. |

Everything below is **emergent** — there is no hard-coded fitness function. The
energy economy alone drives selection.

## The ecosystem (extended mechanics)

### 🌡️ Seasons, climate & migration
Temperature varies with **latitude** (north = top, south = bottom) and the
**season**, which turns every `cycles_per_year` cycles:

- South is hotter than north; summer is hotter everywhere than winter.
- So winter bites hard in the north but is **dampened** in the warm south, while
  summer makes the deep south punishingly hot.
- **Trees migrate** north in summer and south in winter, dragging the food supply
  across the map so nothing can camp one spot forever.
- Living far from a creature's preferred temperature **costs energy** every tick.
  Creatures either chase the comfortable band or **genetically adapt** — the
  `temp_optimum` and `temp_tolerance` genes evolve to match the niche a lineage
  settles into (watch `temp-opt` track the climate over a run).

### 💀 Carcasses & decomposition
The dead **leave a body**. Carcasses are meat for carnivores and scavenging
decomposers; they rot over several cycles, and whatever isn't eaten breaks down
into **fertilizer** in the soil. This opens a genuine scavenger niche that
omnivores reliably colonise.

### 🗡️ Carnivore strategies
Two independent genes let predators evolve distinct "deadly equilibria":

- `lethality` — killing power; a high-lethality hunter punches above its weight
  ("high-kill"), but wastefully, **leaving big carcasses for scavengers**.
- `feed_efficiency` — how thoroughly a kill is consumed; an "efficient" predator
  extracts more and leaves little behind.

### 🌳 Evolving trees
Trees carry their own genome (`poison`, `fruit_energy`, `fertility`,
`temp_optimum`) and evolve by seed:

- Fruit left **on the floor** long enough can **germinate** into a new tree
  (much likelier on fertilized soil), inheriting a mutated parent genome.
- Trees can grow **poisonous** to defend their fruit — eating toxic fruit hurts
  non-resistant creatures, who in turn evolve `poison_resist`. A real arms race.
- **Poop** from well-fed creatures and rotted carcasses fertilizes the ground,
  boosting nearby fruiting and germination.
- Trees die of age / climate misfit, so the forest turns over and its genome
  drifts over time.

## Running it

```bash
cargo run --release                 # animated ASCII view, default settings
cargo run --release -- --no-render  # headless, prints a per-cycle summary
cargo run --release -- --csv run.csv  # export per-cycle H/O/C history
cargo run --release -- --help       # all options
```

Useful flags:

```
--seed N         RNG seed (runs are fully reproducible)
--cycles N       number of cycles to run
--ticks N        ticks per cycle
--width / --height N   world size (height is the north–south climate axis)
--creatures N    starting population
--sources N      starting trees
--delay MS       delay between rendered frames (0 = as fast as possible)
--csv PATH       write per-cycle Herbivore/Omnivore/Carnivore history to CSV
--no-render      headless mode with a per-cycle summary
--log-cycles     print a summary line at each cycle end (with rendering on)
```

### Reading the display

```
H / h   herbivore   (UPPER = asexual-leaning, lower = sexual-leaning)
O / o   omnivore
C / c   carnivore
T       tree        ! = notably poisonous tree
.       fruit
%       carcass
```

The footer reports the season, population by diet class, forest size, and live
evolved averages (diet, temperature optimum, lethality, feed-efficiency, forest
toxicity…). At the end of a run an ASCII **population graph** plots Herbivore /
Omnivore / Carnivore counts over every cycle — the emergent *Red Queen's Race*.

## How a tick works

Each tick (`World::step`) runs these phases:

1. **Season update** — recompute the temperature field from the current cycle.
2. **Drop fruit** — trees fruit based on genome, climate fit and nearby fertilizer.
3. **Sense, move, eat** — creatures head toward prey / carcasses / fruit by diet,
   pay movement + metabolism + **climate** costs, and eat fruit (taking poison
   damage) or carcasses underfoot.
4. **Predation** — hunters kill weaker neighbours, feeding by `feed_efficiency`
   and leaving the rest as a carcass.
5. **Reproduce** — creatures over their energy threshold clone or mate, per the rules.
6. **Droppings** — well-fed creatures deposit fertilizer.

Every `ticks_per_cycle` ticks the **end of cycle** runs: starvation check → age
death roll → tree migration → tree death → carcass decay → fertilizer leaching →
fruit aging & germination → history snapshot.

## Project layout

```
src/
  rng.rs        deterministic SplitMix64 RNG (no deps)
  geometry.rs   2D vector math
  climate.rs    seasonal temperature model + tree migration targets
  config.rs     all tunable parameters + the death-probability rule
  genome.rs     creature traits (diet, climate, toxins, combat), mutation, crossover
  creature.rs   the agents
  food.rs       trees (evolving genome) + fruit
  remains.rs    carcasses + fertilizer
  world.rs      the simulation engine (tick/cycle logic, ecology, stats)
  render.rs     ASCII map, stats footer, and population graph
  main.rs       CLI entry point
  lib.rs        library crate root
```

## Tests

```bash
cargo test
```

31 unit tests cover RNG determinism, geometry, the spec's death-probability and
efficiency formulas, genome/tree mutation bounds, the seasonal climate model
(south warmer, summer warmer, winter dampened in the south, trees migrating with
the seasons), tree migration bounds, and the core reproduction / cycle / history
mechanics.
# JDLV
