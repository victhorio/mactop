use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table,
        canvas::{Canvas, Line as PlotLine, Points},
    },
};

use crate::{
    app::{App, HISTORY_WINDOW, History, HistorySample, Modal, Sort},
    metrics::{CpuTier, GIB, MIB, Pressure, TierKind},
};

const BG: Color = Color::Reset;
const FG: Color = Color::Reset;
const MUTED: Color = Color::DarkGray;
const EDGE: Color = Color::Gray;
const TRACK: Color = Color::Gray;
const GREEN: Color = Color::Green;
const BLUE: Color = Color::Blue;
const PURPLE: Color = Color::Magenta;
const YELLOW: Color = Color::Yellow;
const RED: Color = Color::Red;
const BARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// CPU, memory, and process panels for a viewport of at least 90×25.
pub fn panel_areas(area: Rect, expanded: bool) -> [Rect; 3] {
    let area = Rect::new(area.x + 1, area.y, area.width - 2, area.height);
    if expanded {
        return [Rect::default(), Rect::default(), area];
    }
    let min_process_height = 6;
    let memory_height = (area.height - 9 - min_process_height - 2).min(11);
    let cpu_height = (area.height - memory_height - min_process_height - 2).min(23);
    let cpu = Rect::new(area.x, area.y, area.width, cpu_height);
    let memory = Rect::new(area.x, cpu.bottom() + 1, area.width, memory_height);
    let process_y = memory.bottom() + 1;
    let processes = Rect::new(area.x, process_y, area.width, area.bottom() - process_y);
    [cpu, memory, processes]
}

pub fn help_hint(area: Rect, expanded: bool) -> Rect {
    let [_, _, processes] = panel_areas(area, expanded);
    Rect::new(
        processes.right().saturating_sub(5),
        processes.bottom().saturating_sub(1),
        3,
        1,
    )
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG).fg(FG)), area);
    if area.width < 90 || area.height < 25 {
        frame.render_widget(
            Paragraph::new(
                "mactop\n\nNeeds at least 90 columns × 25 rows.\nResize the terminal; q quits.",
            )
            .style(Style::default().fg(FG)),
            area.inner(ratatui::layout::Margin::new(2, 2)),
        );
        return;
    }

    let [cpu_area, memory_area, process_area] = panel_areas(area, app.expanded);
    if !app.expanded {
        cpu(frame, cpu_area, app);
        memory(frame, memory_area, app);
    }
    processes(frame, process_area, app);
    match app.modal {
        Some(Modal::Help) => help(frame, area, app),
        Some(Modal::Frequency) => frequency(frame, area, app),
        None => {}
    }
}

fn panel(frame: &mut Frame, area: Rect, title: &str, detail: String) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(EDGE))
        .padding(Padding::horizontal(1))
        .title_top(Line::styled(
            format!(" {title} "),
            Style::default().fg(FG).bold(),
        ))
        .title_top(Line::styled(format!(" {detail} "), Style::default().fg(MUTED)).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn cpu(frame: &mut Frame, area: Rect, app: &App) {
    let inner = panel(
        frame,
        area,
        "CPU",
        format!(
            "{} cores · {:.0}% active · {}s",
            app.snapshot.core_count(),
            app.snapshot.cpu_percent(),
            HISTORY_WINDOW.as_secs()
        ),
    );
    let count = app.snapshot.cpu.len();
    let chunks = Layout::horizontal(vec![Constraint::Fill(1); count])
        .spacing(3)
        .split(inner);
    let core_rows = app
        .snapshot
        .cpu
        .iter()
        .zip(chunks.iter())
        .map(|(tier, chunk)| {
            tier.cores
                .len()
                .div_ceil((chunk.width / 22).max(1) as usize) as u16
        })
        .max()
        .unwrap_or(0);
    let detailed = inner.height >= core_rows + 7;
    let chart_height = if detailed {
        inner.height - 4 - core_rows
    } else {
        inner.height - 3
    };
    for (i, tier) in app.snapshot.cpu.iter().enumerate() {
        tier_view(frame, chunks[i], tier, &app.history, chart_height, detailed);
    }
}

fn tier_view(
    frame: &mut Frame,
    area: Rect,
    tier: &CpuTier,
    history: &History,
    chart_height: u16,
    detailed: bool,
) {
    let color = tier_color(tier.kind);
    let narrow = area.width < 55;
    let title = if narrow {
        format!("{}  {} cores", tier.kind.label(), tier.cores.len())
    } else {
        format!(
            "{}  {}  ·  {} cores",
            tier.kind.label(),
            tier.kind.name(),
            tier.cores.len()
        )
    };
    pair(
        frame,
        row(area, 0),
        Line::styled(title, Style::default().fg(color).bold()),
        Line::styled(
            format!("{:.0}%", tier.active_percent()),
            Style::default().fg(FG).bold(),
        ),
    );
    text(
        frame,
        row(area, 1),
        Line::styled(
            format!(
                "{} / {} GHz",
                tier.frequency_ghz
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "—".into()),
                tier.max_frequency_ghz
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "—".into())
            ),
            Style::default().fg(MUTED),
        ),
    );
    chart(
        frame,
        Rect::new(
            area.x,
            area.y + if detailed { 2 } else { 3 },
            area.width,
            chart_height,
        ),
        history,
        100.0,
        color,
        |sample| {
            sample
                .cpu
                .iter()
                .find(|(kind, _)| *kind == tier.kind)
                .unwrap()
                .1
        },
    );
    if detailed {
        let core_columns = (area.width / 22).max(1) as usize;
        let y = area.y + 3 + chart_height;
        let cell_width = area.width / core_columns as u16;
        for (i, value) in tier.cores.iter().enumerate() {
            let cell = Rect::new(
                area.x + (i % core_columns) as u16 * cell_width,
                y + (i / core_columns) as u16,
                cell_width.saturating_sub(1),
                1,
            );
            text(
                frame,
                cell,
                Line::from(vec![
                    Span::styled(format!("{:02} ", i + 1), Style::default().fg(MUTED)),
                    Span::styled(format!("{:>3.0}% ", value), Style::default().fg(FG)),
                ]),
            );
            meter(
                frame,
                Rect::new(cell.x + 8, cell.y, cell.width.saturating_sub(8), 1),
                *value,
                color,
            );
        }
    } else {
        let mut spans = vec![Span::styled("cores ", Style::default().fg(MUTED))];
        for value in &tier.cores {
            spans.push(Span::styled(
                BARS[(value / 100.0 * 8.0).ceil() as usize],
                Style::default().fg(color),
            ));
            spans.push(Span::raw(" "));
        }
        text(frame, row(area, 2), Line::from(spans));
    }
}

fn memory(frame: &mut Frame, area: Rect, app: &App) {
    let mem = &app.snapshot.memory;
    let inner = panel(
        frame,
        area,
        "MEMORY",
        format!(
            "{:.0} GiB · {}",
            mem.total() as f64 / GIB as f64,
            mem.pressure.label()
        ),
    );
    let wide = inner.width >= 140;
    let columns = Layout::horizontal(vec![Constraint::Fill(1); if wide { 3 } else { 2 }])
        .spacing(3)
        .split(inner);
    let overview = columns[0];
    pair(
        frame,
        row(overview, 0),
        Line::styled(
            format!("In use {:.0}%", mem.used_percent()),
            Style::default().fg(FG).bold(),
        ),
        Line::styled(
            format!("{:.1} GiB", mem.used() as f64 / GIB as f64),
            Style::default().fg(FG).bold(),
        ),
    );
    let mut x = overview.x;
    let mut cumulative = 0;
    for (bytes, color, symbol) in [
        (mem.used(), BLUE, "█"),
        (mem.cached, TRACK, "▒"),
        (mem.free, TRACK, "·"),
    ] {
        cumulative += bytes;
        let end = overview.x
            + ((cumulative as f64 / mem.total() as f64) * overview.width as f64).round() as u16;
        text(
            frame,
            Rect::new(x, overview.y + 1, end - x, 1),
            Line::styled(
                symbol.repeat((end - x) as usize),
                Style::default().fg(color),
            ),
        );
        x = end;
    }
    for (offset, name, bytes, style) in [
        (
            2,
            "Available",
            mem.available(),
            Style::default()
                .fg(if mem.pressure == Pressure::Normal {
                    FG
                } else {
                    pressure_color(mem.pressure)
                })
                .bold(),
        ),
        (3, "  Cache", mem.cached, Style::default().fg(MUTED)),
        (4, "  Free", mem.free, Style::default().fg(MUTED)),
        (6, "App", mem.app, Style::default().fg(MUTED)),
        (7, "Wired", mem.wired, Style::default().fg(MUTED)),
        (8, "Compressed", mem.compressed, Style::default().fg(MUTED)),
    ] {
        if offset >= 6 && overview.height < 9 {
            continue;
        }
        pair(
            frame,
            row(overview, offset),
            Line::styled(name, style),
            Line::styled(format!("{:.1} GiB", bytes as f64 / GIB as f64), style),
        );
    }

    let (usage_area, swap_area) = if wide {
        (columns[1], columns[2])
    } else {
        let usage_height = (columns[1].height / 2).min(columns[1].height - 4);
        let rows = Layout::vertical([Constraint::Length(usage_height), Constraint::Fill(1)])
            .split(columns[1]);
        (rows[0], rows[1])
    };
    pair(
        frame,
        row(usage_area, 0),
        Line::styled(
            format!("Usage · {}s", HISTORY_WINDOW.as_secs()),
            Style::default().fg(MUTED),
        ),
        Line::styled("0–100%", Style::default().fg(MUTED)),
    );
    line_chart(
        frame,
        Rect::new(
            usage_area.x,
            usage_area.y + 1,
            usage_area.width,
            usage_area.height - 1,
        ),
        &app.history,
        BLUE,
    );
    swap(frame, swap_area, app);
}

fn swap(frame: &mut Frame, area: Rect, app: &App) {
    let mem = &app.snapshot.memory;
    let idle = mem.swap_in_bytes_per_sec == 0.0 && mem.swap_out_bytes_per_sec == 0.0;
    pair(
        frame,
        row(area, 0),
        Line::styled(
            format!("Swap {:.1} GiB", mem.swap_used as f64 / GIB as f64),
            Style::default().fg(FG).bold(),
        ),
        Line::styled(
            if idle { "Idle" } else { "Paging" },
            Style::default().fg(if idle { MUTED } else { YELLOW }),
        ),
    );
    let peak_rate = app
        .history
        .points()
        .filter(|(x, _)| *x > 0.0)
        .map(|(_, sample)| sample.swap_in.max(sample.swap_out))
        .fold(0.0, f64::max);
    let max_rate = peak_rate.max(MIB as f64);
    let show_scale = area.height >= 5;
    let graph_height = (area.height - 2 - u16::from(show_scale)).min(6);
    let swap_columns = Layout::horizontal([Constraint::Fill(1); 2])
        .spacing(2)
        .split(area);
    for (column, (name, value, color)) in swap_columns.iter().zip([
        ("In", mem.swap_in_bytes_per_sec, GREEN),
        ("Out", mem.swap_out_bytes_per_sec, YELLOW),
    ]) {
        text(
            frame,
            row(*column, 1),
            Line::styled(
                format!("{name}  {}", rate(value)),
                Style::default().fg(if value == 0.0 { MUTED } else { color }),
            ),
        );
        if peak_rate > 0.0 {
            chart(
                frame,
                Rect::new(column.x, area.y + 2, column.width, graph_height),
                &app.history,
                max_rate,
                color,
                |sample| {
                    if name == "In" {
                        sample.swap_in
                    } else {
                        sample.swap_out
                    }
                },
            );
        }
    }
    if peak_rate > 0.0 && show_scale {
        text(
            frame,
            row(area, 2 + graph_height),
            Line::styled(
                format!(
                    "0–{} shared · {}s",
                    rate(max_rate),
                    HISTORY_WINDOW.as_secs()
                ),
                Style::default().fg(MUTED),
            ),
        );
    }
}

fn processes(frame: &mut Frame, area: Rect, app: &App) {
    let mut processes: Vec<_> = app.snapshot.processes.iter().collect();
    processes.sort_by(|a, b| {
        match app.sort {
            Sort::Cpu => b.cpu_percent.total_cmp(&a.cpu_percent),
            Sort::Memory => b.resident_bytes.cmp(&a.resident_bytes),
        }
        .then_with(|| a.pid.cmp(&b.pid))
    });
    let rows = area.height.saturating_sub(3) as usize;
    let offset = app.process_offset.min(processes.len().saturating_sub(rows));
    let detail = format!(
        "{}–{} / {} · {} · p {}",
        if processes.is_empty() { 0 } else { offset + 1 },
        (offset + rows).min(processes.len()),
        processes.len(),
        if app.sort == Sort::Cpu {
            "CPU ↓"
        } else {
            "memory ↓"
        },
        if app.expanded { "collapse" } else { "expand" }
    );
    let inner = panel(frame, area, "PROCESSES", detail);
    let show_share = inner.width >= 118;
    let mut headings = vec!["PID", "PROCESS", "CPU %", "MEMORY"];
    let mut widths = vec![
        Constraint::Length(8),
        Constraint::Fill(1),
        Constraint::Length(10),
        Constraint::Length(13),
    ];
    if show_share {
        headings.push("OF PHYSICAL RAM");
        widths.push(Constraint::Length(26));
    }
    let header = Row::new(headings.iter().map(|heading| Cell::from(*heading)))
        .style(Style::default().fg(MUTED));
    let table_rows = processes.iter().skip(offset).take(rows).map(|process| {
        let mut cells = vec![
            Cell::from(process.pid.to_string()).style(Style::default().fg(MUTED)),
            Cell::from(process.name.clone()).style(Style::default().fg(FG)),
            Cell::from(format!("{:.1}", process.cpu_percent))
                .style(Style::default().fg(if app.sort == Sort::Cpu { BLUE } else { MUTED })),
            Cell::from(memory_size(process.resident_bytes))
                .style(Style::default().fg(if app.sort == Sort::Memory { BLUE } else { FG })),
        ];
        if show_share {
            let percent =
                process.resident_bytes as f64 / app.snapshot.memory.total() as f64 * 100.0;
            let filled = ((percent / 100.0 * 17.0).round() as usize).min(17);
            cells.push(Cell::from(Line::from(vec![
                Span::styled("━".repeat(filled), Style::default().fg(BLUE)),
                Span::styled("━".repeat(17 - filled), Style::default().fg(TRACK)),
                Span::styled(format!(" {:>4.1}%", percent), Style::default().fg(MUTED)),
            ])));
        }
        Row::new(cells)
    });
    frame.render_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(2),
        inner,
    );
    let status = Line::styled(
        if app.sampling_error.is_some() {
            " STALE · collection error · ? ".into()
        } else {
            format!(
                " {} · {} ",
                if app.paused {
                    "PAUSED"
                } else if app.demo_label.is_some() {
                    "MOCK"
                } else {
                    "LIVE"
                },
                app.demo_label.unwrap_or(&app.snapshot.machine)
            )
        },
        Style::default().fg(if app.sampling_error.is_some() {
            RED
        } else if app.paused {
            YELLOW
        } else {
            MUTED
        }),
    );
    text(
        frame,
        Rect::new(area.x + 2, area.bottom() - 1, status.width() as u16, 1),
        status,
    );
    text(
        frame,
        help_hint(frame.area(), app.expanded),
        Line::styled(" ? ", Style::default().fg(FG).bold()),
    );
}

fn popup(
    frame: &mut Frame,
    area: Rect,
    width: u16,
    height: u16,
    title: &str,
    detail: &str,
) -> Rect {
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(Block::default().style(Style::default().bg(BG)), popup);
    panel(frame, popup, title, detail.into())
}

fn help(frame: &mut Frame, area: Rect, app: &App) {
    let mut keys = vec![
        ("p", "Expand / collapse processes"),
        ("c / m", "Sort by CPU / memory"),
        ("↑ ↓ / j k", "Scroll processes"),
        ("space", "Pause / resume"),
        ("f", "Update frequency"),
        ("q / Ctrl-C", "Quit"),
    ];
    if app.demo_label.is_some() {
        keys.extend([("d", "Next demo scenario"), ("t", "Next demo topology")]);
    }
    let mut lines = vec![
        Line::raw(""),
        Line::styled("SHORTCUTS", Style::default().fg(MUTED).bold()),
    ];
    lines.extend(keys.into_iter().map(|(key, description)| {
        Line::from(vec![
            Span::styled(format!("{key:<14}"), Style::default().fg(FG).bold()),
            Span::raw(description),
        ])
    }));
    lines.extend([
        Line::raw(""),
        Line::styled("METRICS", Style::default().fg(MUTED).bold()),
    ]);
    lines.extend(
        [
            ("CPU", "Active time, 0–100%"),
            ("Process CPU", "100% = one core"),
            ("GHz", "Average while active; — = unavailable"),
            ("In use", "App + wired + compressed"),
            ("Available", "Reclaimable cache + free"),
            ("Swap in/out", "Bytes/sec; swap usage is space on disk"),
        ]
        .into_iter()
        .map(|(label, description)| {
            Line::from(vec![
                Span::styled(format!("{label:<14}"), Style::default().fg(MUTED)),
                Span::raw(description),
            ])
        }),
    );
    if let Some(error) = &app.sampling_error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Collection error · showing last good sample",
            Style::default().fg(RED),
        ));
        lines.push(Line::raw(error.as_str()));
    }
    lines.push(Line::raw(""));
    let inner = popup(
        frame,
        area,
        66,
        lines.len() as u16 + 2,
        "HELP",
        "any key closes",
    );
    frame.render_widget(Paragraph::new(lines).style(Style::default().fg(FG)), inner);
}

fn frequency(frame: &mut Frame, area: Rect, app: &App) {
    let inner = popup(frame, area, 42, 8, "UPDATE FREQUENCY", "x close");
    let seconds = app.update_interval.as_secs_f64();
    let period = if seconds < 1.0 {
        format!("Every {} ms", app.update_interval.as_millis())
    } else {
        format!("Every {seconds:.0} s")
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(""),
            Line::styled(
                format!("{:.1} Hz", 1.0 / seconds),
                Style::default().fg(FG).bold(),
            ),
            Line::styled(period, Style::default().fg(MUTED)),
            Line::raw(""),
            Line::styled("+ faster       − slower", Style::default().fg(FG)),
        ])
        .alignment(Alignment::Center),
        inner,
    );
}

fn row(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y + offset,
        area.width,
        if offset < area.height { 1 } else { 0 },
    )
}

fn text(frame: &mut Frame, area: Rect, line: Line<'_>) {
    frame.render_widget(Paragraph::new(line), area);
}

fn pair(frame: &mut Frame, area: Rect, left: Line<'_>, right: Line<'_>) {
    let right_width = (right.width() as u16).min(area.width);
    text(
        frame,
        Rect::new(
            area.x,
            area.y,
            area.width.saturating_sub(right_width + 1),
            area.height,
        ),
        left,
    );
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

fn meter(frame: &mut Frame, area: Rect, percent: f64, color: Color) {
    let filled = (percent / 100.0 * area.width as f64).round() as usize;
    text(
        frame,
        area,
        Line::from(vec![
            Span::styled("━".repeat(filled), Style::default().fg(color)),
            Span::styled(
                "━".repeat(area.width as usize - filled),
                Style::default().fg(TRACK),
            ),
        ]),
    );
}

fn chart(
    frame: &mut Frame,
    area: Rect,
    history: &History,
    max: f64,
    color: Color,
    value: impl Fn(&HistorySample) -> f64,
) {
    if area.is_empty() {
        return;
    }
    let mut peaks = vec![0.0_f64; area.width as usize];
    let mut previous = None;
    let scale = area.width as f64 / HISTORY_WINDOW.as_secs_f64();
    for (x, sample) in history.points() {
        if x > 0.0 {
            let start = if sample.continuous {
                previous.unwrap_or(x)
            } else {
                x
            };
            let first = ((start.max(0.0) * scale).floor() as usize).min(peaks.len() - 1);
            let end = ((x * scale).ceil() as usize)
                .max(first + 1)
                .min(peaks.len());
            // Interval maxima retain brief spikes even in narrow charts.
            for peak in &mut peaks[first..end] {
                *peak = peak.max(value(sample));
            }
        }
        previous = Some(x);
    }
    for (column, value) in peaks.into_iter().enumerate() {
        let eighths = (value / max * area.height as f64 * 8.0).round() as u16;
        for y in 0..area.height {
            let level = eighths.saturating_sub((area.height - 1 - y) * 8).min(8);
            frame.buffer_mut()[(area.x + column as u16, area.y + y)]
                .set_symbol(BARS[level as usize])
                .set_fg(color);
        }
    }
}

fn line_chart(frame: &mut Frame, area: Rect, history: &History, color: Color) {
    if area.is_empty() {
        return;
    }
    let points: Vec<_> = history
        .points()
        .map(|(x, sample)| (x, sample.memory_used, sample.continuous))
        .collect();
    let visible: Vec<_> = points
        .iter()
        .filter(|p| p.0 >= 0.0)
        .map(|p| (p.0, p.1))
        .collect();
    let canvas = Canvas::default()
        .background_color(BG)
        .x_bounds([0.0, HISTORY_WINDOW.as_secs_f64()])
        .y_bounds([0.0, 100.0])
        .paint(|ctx| {
            ctx.draw(&Points::new(&visible, color));
            for pair in points.windows(2) {
                if !pair[1].2 || pair[1].0 < 0.0 {
                    continue;
                }
                let (mut x1, mut y1, _) = pair[0];
                if x1 < 0.0 {
                    y1 += (pair[1].1 - y1) * -x1 / (pair[1].0 - x1);
                    x1 = 0.0;
                }
                ctx.draw(&PlotLine {
                    x1,
                    y1,
                    x2: pair[1].0,
                    y2: pair[1].1,
                    color,
                });
            }
        });
    frame.render_widget(canvas, area);
}

fn tier_color(kind: TierKind) -> Color {
    match kind {
        TierKind::Efficiency => GREEN,
        TierKind::Performance => BLUE,
        TierKind::Super => PURPLE,
    }
}

fn pressure_color(pressure: Pressure) -> Color {
    match pressure {
        Pressure::Normal => GREEN,
        Pressure::Warning => YELLOW,
        Pressure::Critical => RED,
    }
}

fn memory_size(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.0} MiB", bytes as f64 / MIB as f64)
    }
}

fn rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= MIB as f64 {
        format!("{:.1} MiB/s", bytes_per_sec / MIB as f64)
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.0} KiB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}
