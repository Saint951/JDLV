//! Command-line entry point for the Game of Life simulation.

use std::io::Write;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use game_of_life::config::Config;
use game_of_life::render;
use game_of_life::server;
use game_of_life::world::World;

struct Args {
    cfg: Config,
    delay_ms: u64,
    render: bool,
    per_cycle_log: bool,
    csv_path: Option<String>,
    viewer: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut cfg = Config::default();
    let mut delay_ms = 60u64;
    let mut render = true;
    let mut per_cycle_log = false;
    let mut csv_path: Option<String> = None;
    let mut viewer = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let val = |name: &str| -> Result<&String, String> {
            args.get(i + 1)
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let next_f64 = |name: &str| -> Result<f64, String> {
            val(name)?.parse::<f64>().map_err(|e| format!("{name}: {e}"))
        };
        let next_u64 = |name: &str| -> Result<u64, String> {
            val(name)?.parse::<u64>().map_err(|e| format!("{name}: {e}"))
        };
        let next_usize = |name: &str| -> Result<usize, String> {
            val(name)?.parse::<usize>().map_err(|e| format!("{name}: {e}"))
        };

        let mut consumed_value = true;
        match arg.as_str() {
            "--seed"      => cfg.seed                 = next_u64("--seed")?,
            "--cycles"    => cfg.max_cycles           = next_u64("--cycles")?,
            "--ticks"     => cfg.ticks_per_cycle      = next_u64("--ticks")?,
            "--width"     => cfg.width                = next_f64("--width")?,
            "--height"    => cfg.height               = next_f64("--height")?,
            "--creatures" => cfg.initial_creatures    = next_usize("--creatures")?,
            "--sources"   => cfg.initial_food_sources = next_usize("--sources")?,
            "--delay"     => delay_ms                 = next_u64("--delay")?,
            "--csv"       => csv_path                 = Some(val("--csv")?.clone()),
            "--no-render" => {
                render = false;
                consumed_value = false;
            }
            "--log-cycles" => {
                per_cycle_log = true;
                consumed_value = false;
            }
            "--viewer" => {
                viewer = true;
                consumed_value = false;
            }
            "-h" | "--help" => return Err(help()),
            other => return Err(format!("unknown argument: {other}\n\n{}", help())),
        }

        i += if consumed_value { 2 } else { 1 };
    }

    Ok(Args { cfg, delay_ms, render, per_cycle_log, csv_path, viewer })
}

fn help() -> String {
    "Game of Life — an artificial-life simulation.\n\
     \n\
     USAGE:\n\
     \x20 gol [OPTIONS]\n\
     \n\
     OPTIONS:\n\
     \x20 --seed N         RNG seed (default 1)\n\
     \x20 --cycles N       Number of cycles to run (default 40)\n\
     \x20 --ticks N        Ticks per cycle (default 25)\n\
     \x20 --width N        World width (default 80)\n\
     \x20 --height N       World height (default 40)\n\
     \x20 --creatures N    Initial creatures (default 60)\n\
     \x20 --sources N      Initial food sources (default 6)\n\
     \x20 --delay MS       Delay between rendered ticks (default 60)\n\
     \x20 --csv PATH       Write per-cycle H/O/C population history to a CSV file\n\
     \x20 --no-render      Run headless; print a per-cycle summary\n\
     \x20 --log-cycles     Also print a summary line at each cycle end\n\
     \x20 --viewer         Launch the web viewer at http://127.0.0.1:7070\n\
     \x20 -h, --help       Show this help\n\
     \n\
     The goal: a lineage must reproduce enough to still be alive at the end of\n\
     each cycle, where every survivor rolls to die (30% + 5% per cycle survived)."
        .to_string()
}

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(if msg.starts_with("Game of Life") { 0 } else { 2 });
        }
    };

    let cfg = args.cfg.clone();

    println!(
        "Game of Life — seed {}, {} cycles x {} ticks, {} creatures, {} food sources\n",
        cfg.seed,
        cfg.max_cycles,
        cfg.ticks_per_cycle,
        cfg.initial_creatures,
        cfg.initial_food_sources,
    );

    // When --viewer is requested, wrap the world in an Arc<RwLock<>> so the
    // HTTP server thread can read state between ticks.
    let world_arc: Arc<RwLock<World>> = Arc::new(RwLock::new(World::new(cfg.clone())));

    if args.viewer {
        let port = server::spawn(Arc::clone(&world_arc));
        let url  = format!("http://127.0.0.1:{port}");
        println!("🌐 Web viewer running at {url}");
        // Try to open a browser automatically (best-effort, ignore failures).
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn()
            .or_else(|_| std::process::Command::new("open").arg(&url).spawn())
            .or_else(|_| std::process::Command::new("start").arg(&url).spawn());
        println!("   Open {url} in your browser if it didn't open automatically.\n");
    }

    let mut last_cycle = 0u64;
    let mut survived   = true;

    'run: loop {
        let (is_done, extinct) = if let Ok(w) = world_arc.read() {
            (w.cycle >= w.cfg.max_cycles, w.extinct)
        } else {
            (true, true)
        };

        if is_done || extinct {
            if args.viewer {
                std::thread::sleep(Duration::from_millis(100));
                continue 'run;
            } else {
                if extinct {
                    survived = false;
                }
                break 'run;
            }
        }

        // Acquire write lock only for the step.
        let step_ok = world_arc.write().map(|mut w| w.step()).unwrap_or(false);
        if !step_ok {
            if args.viewer {
                std::thread::sleep(Duration::from_millis(100));
                continue 'run;
            } else {
                survived = false;
                break 'run;
            }
        }

        if args.render {
            let frame = world_arc.read().map(|w| render::frame(&w)).unwrap_or_default();
            clear_screen();
            println!("{}", frame);
            let _ = std::io::stdout().flush();
            if args.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(args.delay_ms));
            }
        }

        let cycle = world_arc.read().map(|w| w.cycle).unwrap_or(0);
        if cycle != last_cycle {
            last_cycle = cycle;
            if args.per_cycle_log || !args.render {
                let line = world_arc.read().map(|w| render::summary_line(&w)).unwrap_or_default();
                println!("{}", line);
            }
        }
    }

    // Always show the emergent population graph.
    let pop_graph = world_arc.read().map(|w| render::population_graph(&w, 16)).unwrap_or_default();
    println!("\n{}\n", pop_graph);

    // Optional CSV export.
    if let Some(path) = &args.csv_path {
        let csv = world_arc.read().map(|w| w.history_csv()).unwrap_or_default();
        match std::fs::write(path, csv) {
            Ok(()) => println!("Wrote per-cycle population history to {path}"),
            Err(e) => eprintln!("Failed to write CSV to {path}: {e}"),
        }
    }

    // Final report.
    println!("=================== RUN COMPLETE ===================");
    let (cycle, tick, population) = world_arc.read()
        .map(|w| (w.cycle, w.tick, w.population()))
        .unwrap_or_default();

    if survived && population > 0 {
        let s = world_arc.read().map(|w| w.stats()).unwrap_or_default();
        let (ta, ts, tc, tg, tp) = world_arc.read().map(|w| (
            w.total_asexual_births, w.total_sexual_births,
            w.total_carcasses_spawned, w.total_trees_germinated,
            w.total_poison_deaths,
        )).unwrap_or_default();

        println!(
            "SURVIVED. After {} cycles, {} creatures are alive.",
            cycle, s.population
        );
        println!(
            "Final makeup: {} herbivores / {} omnivores / {} carnivores; \
             {} trees ({} season).",
            s.herbivores, s.omnivores, s.carnivores, s.trees, s.season
        );
        println!(
            "Reproduction: {} asexual, {} sexual births.  \
             Ecology: {} carcasses left, {} trees germinated, {} poison deaths.",
            ta, ts, tc, tg, tp,
        );
        println!(
            "Evolved creatures: diet {:.2}, mating {:.2}, speed {:.2}, sense {:.1}, \
             size {:.2}, aggr {:.2},\n\
             \x20                 temp-opt {:.2}, hardiness {:.2}, poison-resist {:.2}, \
             lethality {:.2}, feed-eff {:.2}, survived {:.1}.",
            s.avg_diet,
            s.avg_mating_pref,
            s.avg_speed,
            s.avg_sense,
            s.avg_size,
            s.avg_aggression,
            s.avg_temp_optimum,
            s.avg_temp_tolerance,
            s.avg_poison_resist,
            s.avg_lethality,
            s.avg_feed_efficiency,
            s.avg_cycles_survived
        );
        println!(
            "Evolved forest: toxicity {:.2}, fertility {:.2}.",
            s.avg_tree_poison, s.avg_tree_fertility
        );
    } else {
        println!(
            "EXTINCT. The population died out at cycle {}, tick {}.",
            cycle, tick
        );
        println!("The lineage failed to reproduce enough to survive. Try another --seed.");
    }
    println!("===================================================");
}
