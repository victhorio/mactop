use std::{io, time::Duration};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
};
use mactop::{
    app::{App, Modal, Sort},
    sampling::{Command, Sampler},
    ui,
};
use ratatui::layout::{Position, Rect};

fn main() -> io::Result<()> {
    let (mut demo, mut sample) = (false, false);
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "mactop — CPU & memory monitor for Apple Silicon\n\nRun without arguments for live native metrics. Press ? for keys.\n  --demo    Use synthetic fixtures instead of native measurements\n  --sample  Print one measured snapshot and exit (no terminal needed)\nMinimum size: 90 columns × 25 rows. No sudo required."
                );
                return Ok(());
            }
            "--demo" => demo = true,
            "--sample" => sample = true,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument {arg}; see --help"),
                ));
            }
        }
    }
    let (sampler, initial) = Sampler::start(demo)?;
    if sample {
        println!("{:#?}", initial.snapshot);
        return Ok(());
    }
    let mut app = App::new(initial.snapshot, initial.at);
    app.demo_label = initial.demo_label;
    let mut terminal = ratatui::try_init()?;
    let result = execute!(io::stdout(), EnableMouseCapture)
        .and_then(|_| run(&mut terminal, &sampler, &mut app));
    let mouse_result = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result.and(mouse_result)
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    sampler: &Sampler,
    app: &mut App,
) -> io::Result<()> {
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| ui::draw(frame, app))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.code == KeyCode::Char('q')
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                    {
                        return Ok(());
                    }
                    if app.modal == Some(Modal::Help) {
                        app.modal = None;
                    } else if app.modal == Some(Modal::Frequency) {
                        match key.code {
                            KeyCode::Char('+' | '=' | '-') => {
                                if let Some(interval) =
                                    app.change_frequency(key.code != KeyCode::Char('-'))
                                {
                                    sampler.command(Command::Interval(interval))?;
                                }
                            }
                            KeyCode::Char('x') | KeyCode::Esc => app.modal = None,
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('?') => app.modal = Some(Modal::Help),
                            KeyCode::Char('f') => app.modal = Some(Modal::Frequency),
                            KeyCode::Char(' ') => app.toggle_pause(),
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
                            KeyCode::Char('d' | 't') if app.demo_label.is_some() => {
                                sampler.command(if key.code == KeyCode::Char('d') {
                                    Command::Scenario
                                } else {
                                    Command::Topology
                                })?;
                            }
                            _ => {}
                        }
                    }
                    dirty = true;
                }
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    if app.modal == Some(Modal::Help) {
                        app.modal = None;
                        dirty = true;
                    } else if app.modal.is_none()
                        && size.width >= 90
                        && size.height >= 25
                        && ui::help_hint(area, app.expanded)
                            .contains(Position::new(mouse.column, mouse.row))
                    {
                        app.modal = Some(Modal::Help);
                        dirty = true;
                    }
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
        if let Some(result) = sampler.take()? {
            dirty |= app.accept(result);
        }
    }
}
