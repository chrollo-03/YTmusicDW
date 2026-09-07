mod app;
mod burn;
mod convert;
mod ui;
mod ytdlp;

use anyhow::{Context, Result};
use app::{App, Screen};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;

fn main() {
    // Double-clicking the exe on Windows spawns a throwaway console that
    // closes the instant the process exits — any eprintln! here is gone
    // before it can be read unless we explicitly pause first.
    if let Err(e) = ytdlp::check_available() {
        eprintln!("yt-dlp is not available on PATH: {e:#}");
        eprintln!("Install it first:");
        eprintln!("  Windows (64-bit): winget install yt-dlp.yt-dlp");
        eprintln!("  Windows (32-bit): download yt-dlp_x86.exe from");
        eprintln!("    https://github.com/yt-dlp/yt-dlp/releases, rename it to yt-dlp.exe, put it on PATH");
        eprintln!("  Linux: apt install yt-dlp (or pip install yt-dlp)");
        pause_before_exit_on_windows();
        std::process::exit(1);
    }

    if let Err(e) = run_app() {
        eprintln!("ytmusicdw exited with an error: {e:#}");
        pause_before_exit_on_windows();
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn pause_before_exit_on_windows() {
    eprintln!("\nPress Enter to close this window...");
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
}

#[cfg(not(windows))]
fn pause_before_exit_on_windows() {}

fn run_app() -> Result<()> {
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    loop {
        app.poll_worker();
        terminal.draw(|f| ui::draw(f, &app))?;

        if app.should_quit {
            return Ok(());
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    app.should_quit = true;
                    continue;
                }
                handle_key(&mut app, key.code);
            }
        }
    }
}

fn handle_key(app: &mut App, code: KeyCode) {
    app.error = None;
    match app.screen {
        Screen::UrlInput => match code {
            KeyCode::Esc => app.should_quit = true,
            KeyCode::Enter => app.start_fetch(),
            KeyCode::Backspace => {
                app.url_input.pop();
            }
            KeyCode::Char(c) => app.url_input.push(c),
            _ => {}
        },
        Screen::TrackList => match code {
            KeyCode::Esc => app.screen = Screen::UrlInput,
            KeyCode::Up => {
                if app.cursor > 0 {
                    app.cursor -= 1;
                }
            }
            KeyCode::Down => {
                if app.cursor + 1 < app.tracks.len() {
                    app.cursor += 1;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(t) = app.tracks.get_mut(app.cursor) {
                    t.selected = !t.selected;
                }
            }
            KeyCode::Char('a') => {
                for t in &mut app.tracks {
                    t.selected = true;
                }
            }
            KeyCode::Char('n') => {
                for t in &mut app.tracks {
                    t.selected = false;
                }
            }
            KeyCode::Char('d') => app.start_download_selected(),
            KeyCode::Char('b') => app.start_burn(),
            KeyCode::Char('c') => app.cycle_disc_capacity(),
            KeyCode::Char(']') => app.inc_target_discs(),
            KeyCode::Char('[') => app.dec_target_discs(),
            KeyCode::Char('0') => app.reset_target_discs(),
            _ => {}
        },
        Screen::Working => {
            if app.awaiting_swap.is_some() && code == KeyCode::Enter {
                app.confirm_disc_swap();
            }
        }
        Screen::Done => match code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Char('n') => {
                app.url_input.clear();
                app.tracks.clear();
                app.log.clear();
                app.screen = Screen::UrlInput;
            }
            _ => {}
        },
    }
}
