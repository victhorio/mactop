use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
};
use mactop::{
    app::{App, HISTORY_LEN, Sort},
    metrics::{MetricsSource, MockSource},
    ui,
};
use ratatui::layout::{Position, Rect};

fn demo(source: &mut MockSource) -> io::Result<App> {
    let mut app = App::new(source.sample()?);
    for _ in 1..HISTORY_LEN {
        app.ingest(source.sample()?);
    }
    app.demo_label = source.label();
    Ok(app)
}

fn main() -> io::Result<()> {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "mactop — CPU & memory monitor (mock data preview)\n\nRun without arguments to open the TUI. Press ? for keys.\nMinimum preview size: 90 columns × 25 rows."
        );
        return Ok(());
    }
    let mut source = MockSource::default();
    let mut app = demo(&mut source)?;
    let mut terminal = ratatui::init();
    let result = execute!(io::stdout(), EnableMouseCapture)
        .and_then(|_| run(&mut terminal, &mut source, &mut app));
    let mouse_result = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result.and(mouse_result)
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    source: &mut MockSource,
    app: &mut App,
) -> io::Result<()> {
    let interval = Duration::from_secs(1);
    let mut next_sample = Instant::now() + interval;
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| ui::draw(frame, app))?;
            dirty = false;
        }
        if event::poll(next_sample.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.code == KeyCode::Char('q')
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                    {
                        return Ok(());
                    }
                    if app.help {
                        app.help = false;
                    } else {
                        match key.code {
                            KeyCode::Char('?') => app.help = true,
                            KeyCode::Char(' ') => app.paused = !app.paused,
                            KeyCode::Char('p') => {
                                app.expanded = !app.expanded;
                                app.process_offset = 0;
                            }
                            KeyCode::Char('c') => app.set_sort(Sort::Cpu),
                            KeyCode::Char('m') => app.set_sort(Sort::Memory),
                            KeyCode::Down | KeyCode::Up | KeyCode::Char('j' | 'k') => {
                                let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
                                let size = terminal.size()?;
                                if size.width >= 90 && size.height >= 25 {
                                    let area = Rect::new(0, 0, size.width, size.height);
                                    let [_, _, processes] = ui::panel_areas(area, app.expanded);
                                    app.scroll(down, processes.height.saturating_sub(3) as usize);
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Char('t') => {
                                if key.code == KeyCode::Char('d') {
                                    source.cycle_scenario();
                                } else {
                                    source.cycle_topology();
                                }
                                let paused = app.paused;
                                let expanded = app.expanded;
                                let sort = app.sort;
                                *app = demo(source)?;
                                app.paused = paused;
                                app.expanded = expanded;
                                app.sort = sort;
                                next_sample = Instant::now() + interval;
                            }
                            _ => {}
                        }
                    }
                    dirty = true;
                }
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    if app.help {
                        app.help = false;
                        dirty = true;
                    } else if size.width >= 90
                        && size.height >= 25
                        && ui::help_hint(area, app.expanded)
                            .contains(Position::new(mouse.column, mouse.row))
                    {
                        app.help = true;
                        dirty = true;
                    }
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
        if Instant::now() >= next_sample {
            if !app.paused {
                app.ingest(source.sample()?);
                dirty = true;
            }
            next_sample = Instant::now() + interval;
        }
    }
}
