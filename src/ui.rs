use std::collections::VecDeque;

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
    app::{App, Sort},
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
            Paragraph::new("mactop\n\nThis preview needs at least 90 columns × 25 rows.\nResize the terminal; q quits.")
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
    if app.help {
        help(frame, area);
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
            "{} cores · {:.0}% active",
            app.snapshot.core_count(),
            app.snapshot.cpu_percent()
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
        tier_view(
            frame,
            chunks[i],
            tier,
            &app.history.cpu[i].1,
            chart_height,
            detailed,
        );
    }
}

fn tier_view(
    frame: &mut Frame,
    area: Rect,
    tier: &CpuTier,
    history: &VecDeque<f64>,
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
                "{:.2} / {:.2} GHz",
                tier.frequency_ghz, tier.max_frequency_ghz
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
        Line::styled("Usage · 60s", Style::default().fg(MUTED)),
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
        &app.history.memory_used,
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
        .swap_in
        .iter()
        .chain(&app.history.swap_out)
        .copied()
        .fold(0.0, f64::max);
    let max_rate = peak_rate.max(MIB as f64);
    let show_scale = area.height >= 5;
    let graph_height = (area.height - 2 - u16::from(show_scale)).min(6);
    let swap_columns = Layout::horizontal([Constraint::Fill(1); 2])
        .spacing(2)
        .split(area);
    for (column, (name, value, history, color)) in swap_columns.iter().zip([
        ("In", mem.swap_in_bytes_per_sec, &app.history.swap_in, GREEN),
        (
            "Out",
            mem.swap_out_bytes_per_sec,
            &app.history.swap_out,
            YELLOW,
        ),
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
                history,
                max_rate,
                color,
            );
        }
    }
    if peak_rate > 0.0 && show_scale {
        text(
            frame,
            row(area, 2 + graph_height),
            Line::styled(
                format!("0–{} shared · 60s", rate(max_rate)),
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
        offset + 1,
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
            let filled = (percent / 100.0 * 17.0).round() as usize;
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
        format!(
            " {} · {} ",
            if app.paused { "PAUSED" } else { "MOCK" },
            app.demo_label
        ),
        Style::default().fg(if app.paused { YELLOW } else { MUTED }),
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

fn help(frame: &mut Frame, area: Rect) {
    let popup = Rect::new(
        area.x + (area.width - 66) / 2,
        area.y + (area.height - 20) / 2,
        66,
        20,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(Block::default().style(Style::default().bg(BG)), popup);
    let inner = panel(frame, popup, "KEYS & METRICS", "any key closes".into());
    let lines = vec![
        Line::styled(
            "A CPU & memory-first monitor. All data is mocked.",
            Style::default().fg(FG).bold(),
        ),
        Line::raw("? / click ? Open help; any key or click closes"),
        Line::raw("p          Expand / collapse processes"),
        Line::raw("c / m      Sort processes by CPU / resident memory"),
        Line::raw("↑ ↓ / j k  Scroll processes"),
        Line::raw("space      Pause / resume sampling"),
        Line::raw("d          Cycle everyday / CPU load / memory pressure"),
        Line::raw("t          Cycle E+P+S / E+P / P+S demo topologies"),
        Line::raw("q / Ctrl-C Quit"),
        Line::raw(""),
        Line::styled(
            "CPU %       Active time, not frequency-scaled capacity",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "Process %   100% is one core; can exceed 100%",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "In use      App + wired + compressed; breakdown in GiB",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "RAM bar     █ in use · ▒ reclaimable cache · · free",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "Available   Cache + free; cache is safe to reclaim",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "Swap in/out Bytes per second, not allocated swap",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "Swap        Disk occupancy; Idle means no current I/O",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "60s → newest · 1s samples · memory chart 0–100%",
            Style::default().fg(MUTED),
        ),
    ];
    frame.render_widget(Paragraph::new(lines).style(Style::default().fg(FG)), inner);
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

fn chart(frame: &mut Frame, area: Rect, history: &VecDeque<f64>, max: f64, color: Color) {
    if area.is_empty() || history.is_empty() {
        return;
    }
    for column in 0..area.width {
        let value = history_peak(history, column, area.width);
        let eighths = (value / max * area.height as f64 * 8.0).round() as u16;
        for y in 0..area.height {
            let level = eighths.saturating_sub((area.height - 1 - y) * 8).min(8);
            let symbol = if level == 0 && y == area.height - 1 {
                "─"
            } else {
                BARS[level as usize]
            };
            frame.buffer_mut()[(area.x + column, area.y + y)]
                .set_symbol(symbol)
                .set_fg(if level == 0 { TRACK } else { color });
        }
    }
}

fn line_chart(frame: &mut Frame, area: Rect, history: &VecDeque<f64>, color: Color) {
    if area.is_empty() || history.is_empty() {
        return;
    }
    let points: Vec<_> = history
        .iter()
        .enumerate()
        .map(|(i, value)| (i as f64, *value))
        .collect();
    let canvas = Canvas::default()
        .background_color(BG)
        .x_bounds([0.0, (history.len().max(2) - 1) as f64])
        .y_bounds([0.0, 100.0])
        .paint(|ctx| {
            ctx.draw(&Points::new(&points, color));
            for pair in points.windows(2) {
                ctx.draw(&PlotLine {
                    x1: pair[0].0,
                    y1: pair[0].1,
                    x2: pair[1].0,
                    y2: pair[1].1,
                    color,
                });
            }
        });
    frame.render_widget(canvas, area);
}

fn history_peak(history: &VecDeque<f64>, column: u16, width: u16) -> f64 {
    // Bucket maxima preserve brief spikes when the history is narrower than 60 cells.
    let start = column as usize * history.len() / width as usize;
    let end = ((column as usize + 1) * history.len() / width as usize).max(start + 1);
    history
        .iter()
        .skip(start)
        .take(end - start)
        .copied()
        .fold(0.0, f64::max)
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
