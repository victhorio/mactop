use mactop::{
    app::{App, Sort},
    metrics::{CpuTier, GIB, MIB, MetricsSource, MockSource, Pressure, Process, TierKind},
    ui,
};
use ratatui::{Terminal, backend::TestBackend, style::Color};

const SIZES: [(u16, u16); 7] = [
    (180, 54),
    (90, 54),
    (90, 25),
    (184, 25),
    (90, 40),
    (90, 47),
    (90, 48),
];

fn render(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn supported_sizes_show_only_available_tiers_and_keep_primary_metrics() {
    let mut source = MockSource::default();
    for labels in [[true, true, true], [true, true, false], [false, true, true]] {
        let mut app = App::new(source.sample().unwrap());
        for (width, height) in SIZES {
            let screen = render(&app, width, height);
            assert!(screen.lines().next().unwrap().contains("CPU"));
            let bottom = screen.lines().last().unwrap();
            assert!(bottom.contains('└') && bottom.contains('?') && bottom.contains('┘'));
            let memory_y = screen
                .lines()
                .position(|line| line.contains("MEMORY"))
                .unwrap();
            let process_y = screen
                .lines()
                .position(|line| line.contains("PROCESSES"))
                .unwrap();
            assert!(
                process_y - memory_y - 1 <= 11,
                "{width}×{height}: memory must not stretch beyond its content\n{screen}"
            );
            if height >= 40 {
                assert!(screen.contains("Compressed"));
            }
            assert!(!screen.contains("CPU & memory"));
            assert!(!screen.contains("space pause"));
            for (label, present) in ["E-CPU", "P-CPU", "S-CPU"].into_iter().zip(labels) {
                assert_eq!(screen.contains(label), present, "{width}×{height}: {label}");
                if present {
                    assert!(
                        screen.lines().nth(1).unwrap().contains(label),
                        "{width}×{height}: CPU tiers must be side by side"
                    );
                }
            }
            for label in [
                "Available",
                "Normal",
                "In use",
                "Cache",
                "Free",
                "Swap",
                "In  ",
                "Out  ",
                "PROCESSES",
                "OrbStack",
            ] {
                assert!(
                    screen.contains(label),
                    "{width}×{height} lost {label}\n{screen}"
                );
            }
            app.expanded = true;
            let expanded = render(&app, width, height);
            assert!(expanded.lines().next().unwrap().contains("PROCESSES"));
            assert!(expanded.lines().last().unwrap().contains('?'));
            assert!(expanded.contains("launchd"));
            assert!(expanded.contains("collapse"));
            app.help = true;
            let help = render(&app, width, height);
            assert!(help.contains("KEYS & METRICS"));
            assert!(help.contains("Swap in/out"));
            app.help = false;
            app.expanded = false;
        }
        source.cycle_topology();
    }
}

#[test]
fn in_use_headroom_and_swap_activity_are_independent() {
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.memory.app = 16 * GIB;
    snapshot.memory.wired = 4 * GIB;
    snapshot.memory.compressed = 4 * GIB;
    snapshot.memory.cached = 6 * GIB;
    snapshot.memory.free = 2 * GIB;
    snapshot.memory.swap_used = 2 * GIB;
    snapshot.memory.swap_allocated = 5 * GIB;
    snapshot.memory.swap_in_bytes_per_sec = 0.0;
    snapshot.memory.swap_out_bytes_per_sec = 3.5 * MIB as f64;
    for (app_gib, cache_gib, in_use, used_size, available) in [
        (16, 6, "In use 75%", "24.0 GiB", "8.0 GiB"),
        (11, 11, "In use 59%", "19.0 GiB", "13.0 GiB"),
    ] {
        snapshot.memory.app = app_gib * GIB;
        snapshot.memory.cached = cache_gib * GIB;
        for pressure in [Pressure::Normal, Pressure::Critical] {
            snapshot.memory.pressure = pressure;
            let app = App::new(snapshot.clone());
            for (width, height) in SIZES {
                let screen = render(&app, width, height);
                for expected in [
                    in_use,
                    used_size,
                    "Swap 2.0 GiB",
                    "Paging",
                    "In  0 B/s",
                    "Out  3.5 MiB/s",
                    pressure.label(),
                ] {
                    assert!(
                        screen.contains(expected),
                        "{width}×{height} lost {expected}\n{screen}"
                    );
                }
                assert!(
                    screen
                        .lines()
                        .any(|line| line.contains("Available") && line.contains(available)),
                    "{width}×{height}: wrong headroom\n{screen}"
                );
                assert!(
                    screen
                        .lines()
                        .any(|line| line.contains(in_use) && line.contains(used_size)),
                    "{width}×{height}: in-use value must exclude cache\n{screen}"
                );
            }
        }
    }
}

#[test]
fn combined_cpu_is_weighted_by_core_count_not_tier_count() {
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.cpu = vec![
        CpuTier {
            kind: TierKind::Efficiency,
            cores: vec![0.0],
            frequency_ghz: 1.0,
            max_frequency_ghz: 2.0,
        },
        CpuTier {
            kind: TierKind::Super,
            cores: vec![100.0; 4],
            frequency_ghz: 4.0,
            max_frequency_ghz: 4.5,
        },
    ];
    let app = App::new(snapshot);
    for (width, height) in SIZES {
        let screen = render(&app, width, height);
        assert!(screen.contains("5 cores · 80% active"));
        assert!(!screen.contains("P-CPU"));
    }
}

#[test]
fn narrow_memory_views_keep_usage_and_recent_swap_activity() {
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.memory.app = 8 * GIB;
    snapshot.memory.wired = 4 * GIB;
    snapshot.memory.compressed = 4 * GIB;
    snapshot.memory.cached = 14 * GIB;
    snapshot.memory.free = 2 * GIB;
    snapshot.memory.swap_in_bytes_per_sec = MIB as f64;
    snapshot.memory.swap_out_bytes_per_sec = 3.5 * MIB as f64;
    let mut app = App::new(snapshot.clone());
    snapshot.memory.swap_in_bytes_per_sec = 0.0;
    snapshot.memory.swap_out_bytes_per_sec = 0.0;
    app.ingest(snapshot);
    for (width, height) in [(90, 25), (90, 54)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let lines: Vec<_> = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        let memory_history_y = lines
            .iter()
            .position(|line| line.contains("Usage · 60s"))
            .unwrap() as u16
            + 1;
        let swap_y = lines.iter().position(|line| line.contains("Swap")).unwrap() as u16;
        assert!(
            (memory_history_y..swap_y).any(|y| (0..width).any(|x| {
                let cell = &buffer[(x, y)];
                cell.fg == Color::Blue
                    && cell
                        .symbol()
                        .chars()
                        .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
            })),
            "{width}×{height}: missing memory history"
        );
        let history_rows = |color, start, end| {
            (start..end)
                .filter(|&y| {
                    (0..width).any(|x| {
                        let cell = &buffer[(x, y)];
                        cell.fg == color
                            && matches!(
                                cell.symbol(),
                                "▁" | "▂" | "▃" | "▄" | "▅" | "▆" | "▇" | "█"
                            )
                    })
                })
                .count()
        };
        let in_rows = history_rows(Color::Green, swap_y + 2, height);
        let out_rows = history_rows(Color::Yellow, swap_y + 2, height);
        assert!(in_rows > 0);
        assert!(
            out_rows > in_rows,
            "the 3.5 MiB/s out spike must look larger than the 1 MiB/s in spike on their shared scale"
        );
        assert!(lines.iter().any(|line| line.contains("In  0 B/s")));
        assert!(lines.iter().any(|line| line.contains("Out  0 B/s")));
        assert!(lines.iter().any(|line| line.contains("Idle")));
        if height == 54 {
            assert!(lines.iter().any(|line| line.contains("0–3.5 MiB/s shared")));
        }
    }
}

#[test]
fn compact_process_view_sorts_and_scrolls_without_losing_rows() {
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.processes = (0..32)
        .map(|i| Process {
            pid: i,
            name: format!("fixture-{i:02}"),
            cpu_percent: i as f64 * 13.0,
            resident_bytes: (32 - i) as u64 * GIB,
        })
        .collect();
    for (width, height, visible) in [(90, 25, 3), (184, 25, 3), (90, 54, 15), (180, 54, 15)] {
        let mut app = App::new(snapshot.clone());
        let screen = render(&app, width, height);
        assert!(screen.contains(&format!("1–{visible} / 32")));
        assert!(screen.contains("fixture-31"));
        assert!(screen.contains(&format!("fixture-{:02}", 32 - visible)));
        assert!(!screen.contains(&format!("fixture-{:02}", 31 - visible)));
        app.set_sort(Sort::Memory);
        let screen = render(&app, width, height);
        assert!(screen.contains("fixture-00"));
        assert!(screen.contains(&format!("fixture-{:02}", visible - 1)));
        assert!(!screen.contains(&format!("fixture-{visible:02}")));
        for _ in 0..32 {
            app.scroll(true, visible);
        }
        let screen = render(&app, width, height);
        assert!(screen.contains("fixture-31"));
        assert!(screen.contains(&format!("fixture-{:02}", 32 - visible)));
        assert!(!screen.contains(&format!("fixture-{:02}", 31 - visible)));
        app.scroll(false, visible);
        let screen = render(&app, width, height);
        assert!(screen.contains(&format!("fixture-{:02}", 31 - visible)));
        assert!(!screen.contains("fixture-31"));
        app.expanded = true;
        app.process_offset = 0;
        let screen = render(&app, width, height);
        let expanded_rows = (height as usize - 3).min(32);
        assert!(screen.contains("fixture-00"));
        assert!(screen.contains(&format!("fixture-{:02}", expanded_rows - 1)));
        if expanded_rows < 32 {
            assert!(!screen.contains(&format!("fixture-{expanded_rows:02}")));
        }
    }
}

#[test]
fn dashboard_and_overlays_inherit_the_terminal_theme() {
    let mut source = MockSource::default();
    for _ in 0..3 {
        let mut app = App::new(source.sample().unwrap());
        for (expanded, help) in [(false, false), (true, false), (false, true)] {
            app.expanded = expanded;
            app.help = help;
            for (width, height) in [(180, 54), (90, 25)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
                let buffer = terminal.backend().buffer();
                assert_eq!(
                    buffer[(3, 0)].fg,
                    Color::Reset,
                    "primary text must inherit the terminal foreground"
                );
                for cell in &buffer.content {
                    assert_eq!(
                        cell.bg,
                        Color::Reset,
                        "do not paint over the terminal background"
                    );
                    assert!(
                        !matches!(cell.fg, Color::Rgb(..) | Color::Indexed(_)),
                        "accents must use the terminal's ANSI palette"
                    );
                }
            }
        }
        source.cycle_scenario();
    }
}
