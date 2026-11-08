use std::{
    io,
    time::{Duration, Instant},
};

use mactop::{
    app::{App, HISTORY_WINDOW, Modal, Sort},
    metrics::{CpuTier, GIB, MIB, MetricsSource, MockSource, Pressure, Process, TierKind},
    sampling::Update,
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
        let mut app = App::new(source.sample().unwrap(), Instant::now());
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
            app.modal = Some(Modal::Help);
            let help = render(&app, width, height);
            assert!(help.contains("SHORTCUTS"));
            assert!(help.contains("Swap in/out"));
            app.modal = None;
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
            let app = App::new(snapshot.clone(), Instant::now());
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
            frequency_ghz: Some(1.0),
            max_frequency_ghz: Some(2.0),
        },
        CpuTier {
            kind: TierKind::Super,
            cores: vec![100.0; 4],
            frequency_ghz: Some(4.0),
            max_frequency_ghz: Some(4.5),
        },
    ];
    let app = App::new(snapshot, Instant::now());
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
    let start = Instant::now();
    let mut app = App::new(snapshot.clone(), start);
    snapshot.memory.swap_in_bytes_per_sec = 0.0;
    snapshot.memory.swap_out_bytes_per_sec = 0.0;
    app.ingest(snapshot, start + Duration::from_secs(1), true);
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
            name: format!("fixture-{i:02}").into(),
            cpu_percent: i as f64 * 13.0,
            resident_bytes: (32 - i) as u64 * GIB,
        })
        .collect();
    for (width, height, visible) in [(90, 25, 3), (184, 25, 3), (90, 54, 15), (180, 54, 15)] {
        let mut app = App::new(snapshot.clone(), Instant::now());
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
        let mut app = App::new(source.sample().unwrap(), Instant::now());
        for (expanded, modal) in [
            (false, None),
            (true, None),
            (false, Some(Modal::Help)),
            (false, Some(Modal::Frequency)),
        ] {
            app.expanded = expanded;
            app.modal = modal;
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

#[test]
fn live_view_does_not_invent_history_clocks_or_successful_samples() {
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.cpu.truncate(1);
    snapshot.cpu[0].cores.fill(75.0);
    snapshot.cpu[0].frequency_ghz = None;
    snapshot.cpu[0].max_frequency_ghz = None;
    snapshot.processes.clear();
    let start = Instant::now();
    let mut app = App::new(snapshot.clone(), start);
    let cold = render(&app, 180, 54);
    assert!(cold.contains("LIVE"));
    assert!(cold.contains("— / — GHz"));
    assert!(!cold.contains("0.00 GHz"));
    assert!(cold.contains("0–0 / 0"));
    let bars = |screen: &str| screen.chars().filter(|c| matches!(c, '▁'..='█')).count();
    for second in 1..HISTORY_WINDOW.as_secs() {
        app.ingest(snapshot.clone(), start + Duration::from_secs(second), true);
    }
    let warm = render(&app, 180, 54);
    assert!(
        bars(&warm) > 3 * bars(&cold),
        "one sample must not stretch into a full minute of history"
    );
    app.sampling_error = Some("native sampling failed".into());
    let stale = render(&app, 90, 25);
    assert!(stale.contains("STALE"));
    assert!(!stale.contains("LIVE"));
    assert!(
        stale.contains("In use"),
        "keep the last good sample visible"
    );
    app.modal = Some(Modal::Help);
    assert!(render(&app, 90, 25).contains("native sampling failed"));
}

#[test]
fn cpu_history_has_no_floating_zero_ticks() {
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.cpu.truncate(2);
    for tier in &mut snapshot.cpu {
        tier.cores.fill(0.0);
    }
    let start = Instant::now();
    let mut app = App::new(snapshot.clone(), start);
    let mut at = start;
    for value in [0.0, 3.0, 24.0] {
        snapshot.cpu[1].cores.fill(value);
        for _ in 0..20 {
            at += Duration::from_secs(1);
            app.ingest(snapshot.clone(), at, true);
        }
    }
    let mut terminal = Terminal::new(TestBackend::new(90, 25)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut has_small_bar = false;
    let mut has_full_bar = false;
    for y in 4..8 {
        for x in 3..87 {
            let cell = &buffer[(x, y)];
            assert_ne!(
                cell.symbol(),
                "─",
                "a centered dash is not a zero-height bar"
            );
            if cell.fg == Color::Blue {
                has_small_bar |= cell.symbol() == "▁";
                has_full_bar |= cell.symbol() == "█";
            }
        }
    }
    assert!(
        has_small_bar && has_full_bar,
        "keep low activity and spikes visible"
    );
    for y in 4..8 {
        for x in 3..42 {
            assert_eq!(buffer[(x, y)].symbol(), " ", "idle E-CPU history is empty");
        }
    }
}

#[test]
fn frequency_changes_preserve_history_and_its_time_window() {
    let start = Instant::now();
    let mut app = App::new(MockSource::default().sample().unwrap(), start);
    for second in 1..20 {
        app.ingest(
            app.snapshot.clone(),
            start + Duration::from_secs(second),
            true,
        );
    }
    let original: Vec<_> = SIZES.iter().map(|&(w, h)| render(&app, w, h)).collect();
    for (faster, interval, frequency, period) in [
        (true, 500, "2.0 Hz", "Every 500 ms"),
        (true, 250, "4.0 Hz", "Every 250 ms"),
        (false, 500, "2.0 Hz", "Every 500 ms"),
        (false, 1000, "1.0 Hz", "Every 1 s"),
        (false, 2000, "0.5 Hz", "Every 2 s"),
        (false, 5000, "0.2 Hz", "Every 5 s"),
    ] {
        assert_eq!(
            app.change_frequency(faster),
            Some(Duration::from_millis(interval))
        );
        for ((width, height), before) in SIZES.into_iter().zip(&original) {
            app.modal = None;
            assert_eq!(
                &render(&app, width, height),
                before,
                "rate changes must not erase or rescale history"
            );
            assert!(before.contains("Usage · 60s"));
            app.modal = Some(Modal::Frequency);
            let screen = render(&app, width, height);
            for label in [
                "UPDATE FREQUENCY",
                frequency,
                period,
                "+ faster",
                "− slower",
                "x close",
            ] {
                assert!(screen.contains(label), "{width}×{height}: missing {label}");
            }
        }
        if interval == 250 {
            assert_eq!(app.change_frequency(true), None);
        }
    }
    assert_eq!(app.change_frequency(false), None);
}

#[test]
fn timestamped_charts_preserve_spikes_leave_gaps_and_expire_by_age() {
    let start = Instant::now();
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.cpu.truncate(1);
    snapshot.cpu[0].cores.fill(0.0);
    snapshot.memory.app = 8 * GIB;
    snapshot.memory.wired = 4 * GIB;
    snapshot.memory.compressed = 4 * GIB;
    snapshot.memory.cached = 14 * GIB;
    snapshot.memory.free = 2 * GIB;
    snapshot.memory.swap_in_bytes_per_sec = 0.0;
    snapshot.memory.swap_out_bytes_per_sec = 0.0;
    let mut app = App::new(snapshot.clone(), start);
    for (millis, activity, swap, continuous) in [
        (10_000, 100.0, MIB as f64, true),
        (10_250, 0.0, 0.0, true),
        (40_000, 50.0, 0.0, false),
        (45_000, 50.0, 0.0, true),
    ] {
        snapshot.cpu[0].cores.fill(activity);
        snapshot.memory.swap_in_bytes_per_sec = swap;
        app.ingest(
            snapshot.clone(),
            start + Duration::from_millis(millis),
            continuous,
        );
    }
    let screen = render(&app, 90, 25);
    let rows: Vec<Vec<_>> = screen.lines().map(|line| line.chars().collect()).collect();
    // The 84-column CPU plot covers −15…45 seconds, not five equal sample slots.
    assert_eq!(rows[4][28], '█', "retain the ten-second full-load interval");
    assert_eq!(rows[4][81], ' ', "the new 50% interval is only half height");
    assert_eq!(rows[7][81], '█');
    for cell in &rows[7][43..70] {
        assert_eq!(*cell, ' ', "do not fill a missing CPU interval");
    }
    let memory_y = screen
        .lines()
        .position(|line| line.contains("Usage ·"))
        .unwrap()
        + 1;
    assert!(
        rows[memory_y]
            .iter()
            .any(|c| ('\u{2801}'..='\u{28ff}').contains(c))
    );
    for cell in &rows[memory_y][67..78] {
        assert_eq!(*cell, ' ', "do not connect memory across a gap");
    }
    assert!(render(&app, 180, 54).contains("0–1.0 MiB/s shared · 60s"));

    for millis in [70_000, 70_500] {
        app.ingest(
            snapshot.clone(),
            start + Duration::from_millis(millis),
            true,
        );
        let expired = render(&app, 90, 25);
        assert!(
            expired.lines().nth(4).unwrap().chars().all(|c| c != '█'),
            "the old CPU spike expired"
        );
        assert!(
            !render(&app, 180, 54).contains("shared ·"),
            "expired paging must not keep the swap chart alive"
        );
    }

    for (millis, activity) in [(75_000, 0.0), (75_250, 100.0), (75_500, 0.0), (76_000, 0.0)] {
        snapshot.cpu[0].cores.fill(activity);
        app.ingest(
            snapshot.clone(),
            start + Duration::from_millis(millis),
            true,
        );
    }
    let spike = render(&app, 90, 25);
    assert_eq!(
        spike.lines().nth(4).unwrap().chars().nth(85),
        Some('█'),
        "a quarter-second peak must survive a later zero reading in the same chart column"
    );
}

#[test]
fn dense_and_mixed_rate_histories_render_the_same_elapsed_window() {
    let start = Instant::now();
    let mut snapshot = MockSource::default().sample().unwrap();
    for tier in &mut snapshot.cpu {
        tier.cores.fill(75.0);
    }
    snapshot.memory.swap_in_bytes_per_sec = MIB as f64;
    snapshot.memory.swap_out_bytes_per_sec = 3.5 * MIB as f64;
    let mut dense = App::new(snapshot.clone(), start);
    let mut mixed = App::new(snapshot.clone(), start);
    for step in 1..=325 {
        dense.ingest(
            snapshot.clone(),
            start + Duration::from_millis(step * 250),
            true,
        );
    }
    for millis in [
        500, 1_000, 2_000, 4_000, 9_000, 14_000, 19_000, 24_000, 29_000, 34_000, 39_000, 44_000,
        49_000, 54_000, 59_000, 64_000, 69_000, 74_000, 79_000, 81_250,
    ] {
        mixed.ingest(
            snapshot.clone(),
            start + Duration::from_millis(millis),
            true,
        );
    }
    for (width, height) in SIZES {
        assert_eq!(
            render(&dense, width, height),
            render(&mixed, width, height),
            "elapsed time, not sample count, sets the coverage at {width}×{height}"
        );
    }
}

#[test]
fn accepted_updates_preserve_history_and_break_only_missing_intervals() {
    let start = Instant::now();
    let mut snapshot = MockSource::default().sample().unwrap();
    snapshot.cpu.truncate(1);
    snapshot.cpu[0].cores.fill(100.0);
    let update = |sequence, second, delayed| Update {
        snapshot: snapshot.clone(),
        demo_label: None,
        reset: false,
        sequence,
        at: start + Duration::from_secs(second),
        delayed,
    };
    for interruption in [
        "none",
        "skipped",
        "delayed",
        "error",
        "pause",
        "quick pause",
    ] {
        let mut app = App::new(snapshot.clone(), start);
        assert!(app.accept(Ok(update(1, 10, false))));
        let mut sequence = 2;
        match interruption {
            "skipped" => sequence = 3,
            "error" => {
                assert!(app.accept(Err(io::Error::other("collection failed"))));
                assert!(render(&app, 90, 25).contains("STALE"));
                assert_eq!(app.history.samples.len(), 2);
                sequence = 3;
            }
            "pause" | "quick pause" => {
                app.toggle_pause();
                let frozen = render(&app, 90, 25);
                if interruption == "pause" {
                    assert!(!app.accept(Ok(update(2, 15, false))));
                    assert_eq!(render(&app, 90, 25), frozen);
                    sequence = 3;
                }
                app.toggle_pause();
            }
            _ => {}
        }
        assert!(app.accept(Ok(update(sequence, 20, interruption == "delayed"))));
        assert_eq!(
            app.history.samples.len(),
            3,
            "{interruption}: keep earlier history"
        );
        assert_eq!(app.history.samples.front().unwrap().at, start);
        let screen = render(&app, 90, 25);
        assert!(screen.contains("LIVE") && !screen.contains("STALE"));
        let cpu: Vec<_> = screen.lines().nth(7).unwrap().chars().collect();
        assert!(
            cpu[65..72].iter().all(|c| *c == '█'),
            "keep the first measured interval"
        );
        // At t=20, these columns cover the interior of t=10…20, not its endpoints.
        let expected = if interruption == "none" { '█' } else { ' ' };
        assert!(
            cpu[75..85].iter().all(|c| *c == expected),
            "{interruption}: wrong connection across the latest interval\n{screen}"
        );
    }
}

#[test]
fn explicit_demo_reset_works_while_paused_without_resetting_the_view() {
    let start = Instant::now();
    let snapshot = MockSource::default().sample().unwrap();
    let mut app = App::new(snapshot.clone(), start);
    app.ingest(snapshot.clone(), start + Duration::from_secs(1), true);
    app.expanded = true;
    app.set_sort(Sort::Memory);
    app.process_offset = 2;
    app.modal = Some(Modal::Frequency);
    app.toggle_pause();
    assert!(app.accept(Ok(Update {
        snapshot,
        demo_label: Some("CPU load"),
        reset: true,
        sequence: 4,
        at: start + Duration::from_secs(2),
        delayed: false,
    })));
    assert_eq!(app.history.samples.len(), 1);
    assert_eq!(app.process_offset, 0);
    assert!(app.paused && app.expanded);
    assert_eq!(app.sort, Sort::Memory);
    assert_eq!(app.modal, Some(Modal::Frequency));
    assert_eq!(app.demo_label, Some("CPU load"));
    assert!(render(&app, 90, 25).contains("UPDATE FREQUENCY"));
}
