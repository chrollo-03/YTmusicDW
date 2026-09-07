use crate::app::{App, Screen, TrackStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    draw_header(f, chunks[0], app);

    match app.screen {
        Screen::UrlInput => draw_url_input(f, chunks[1], app),
        Screen::TrackList => draw_track_list(f, chunks[1], app),
        Screen::Working => draw_log(f, chunks[1], app),
        Screen::Done => draw_done(f, chunks[1]),
    }

    draw_footer(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = " ytmusicdw — YouTube playlist -> CD-audio -> burn ";
    let mut spans = vec![Span::raw(title)];
    if let Some(err) = &app.error {
        spans.push(Span::styled(format!("  ERROR: {err}"), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
    }
    let p = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_url_input(f: &mut Frame, area: Rect, app: &App) {
    let text = vec![
        Line::from("Paste a YouTube playlist (or single video) URL:"),
        Line::from(""),
        Line::from(Span::styled(format!("> {}", app.url_input), Style::default().fg(Color::Cyan))),
        Line::from(""),
        Line::from(Span::styled(
            if app.busy { "Fetching..." } else { "Enter: fetch   Esc: quit" },
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(text)
        .block(Block::default().title(" Playlist URL ").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_track_list(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(4)])
        .split(area);

    // Precompute once per frame: which disc each track index lands on.
    let disc_groups = app.disc_groups_indices();
    let mut disc_of: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (disc_i, group) in disc_groups.iter().enumerate() {
        for &idx in group {
            disc_of.insert(idx, disc_i + 1);
        }
    }

    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let checkbox = if t.selected { "[x]" } else { "[ ]" };
            let disc_tag = match disc_of.get(&i) {
                Some(d) => format!("CD{d}"),
                None => "  ".to_string(),
            };
            let status = match &t.status {
                TrackStatus::Pending => String::new(),
                TrackStatus::Downloading => " (downloading...)".to_string(),
                TrackStatus::Downloaded(_) => " (downloaded)".to_string(),
                TrackStatus::Failed(reason) => format!(" (FAILED: {})", truncate(reason, 40)),
            };
            let label = format!("{} — {}", t.track.title, t.track.artist_label());
            let line = format!(
                "{checkbox} {disc_tag:<4}{:>3}  {:<58} {:>6}{status}",
                i + 1,
                truncate(&label, 58),
                t.track.duration_label()
            );
            let style = match t.status {
                TrackStatus::Failed(_) => Style::default().fg(Color::Red),
                TrackStatus::Downloaded(_) => Style::default().fg(Color::Green),
                _ => Style::default(),
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title(" Tracks ").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    state.select(Some(app.cursor));
    f.render_stateful_widget(list, chunks[0], &mut state);

    let total = app.total_selected_seconds();
    let selected_n = app.tracks.iter().filter(|t| t.selected).count();
    let n_discs = disc_groups.len().max(1);
    let cap_min = app.disc_capacity_secs / 60;
    let eff = app.effective_capacity_secs();
    let line1 = format!(
        "{selected_n}/{} tracks selected · {}:{:02} total · needs {n_discs} disc(s) @ {cap_min}min ('c' to change size)",
        app.tracks.len(),
        total / 60,
        total % 60
    );

    let (line2, line2_color) = match app.over_target_budget() {
        Some((needed, over)) => (
            format!(
                "target: {} disc(s) — needs {needed}, OVER by {}:{:02}. Remove tracks to fit ('[' ']' adjust target, '0' auto)",
                app.target_disc_count.unwrap(),
                over / 60,
                over % 60
            ),
            Color::Red,
        ),
        None => {
            let target_label = match app.target_disc_count {
                Some(n) => format!("target: {n} disc(s), fits"),
                None => "target: auto".to_string(),
            };
            (
                format!(
                    "{target_label} · effective budget {}:{:02}/disc ({cap_min}min minus 2s track gaps + 30s safety margin) · '[' ']' set target, '0' auto",
                    eff / 60,
                    eff % 60
                ),
                Color::Green,
            )
        }
    };

    let p = Paragraph::new(vec![
        Line::from(Span::styled(line1, Style::default().fg(Color::Cyan))),
        Line::from(Span::styled(line2, Style::default().fg(line2_color))),
    ])
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(p, chunks[1]);
}

fn draw_log(f: &mut Frame, area: Rect, app: &App) {
    if let Some((next_disc, total)) = app.awaiting_swap {
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("Disc {}/{total} burned.", next_disc - 1),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!("Insert a fresh blank CD-R for disc {next_disc}/{total}, then press Enter.")),
        ];
        let p = Paragraph::new(text)
            .block(Block::default().title(" Swap disc ").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
        return;
    }

    let lines: Vec<Line> = app.log.iter().rev().take(area.height as usize).rev().map(|s| Line::from(s.as_str())).collect();
    let title = if app.busy { " Working... " } else { " Log " };
    let p = Paragraph::new(lines)
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_done(f: &mut Frame, area: Rect) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled("Disc(s) burned. Go play them in the car.", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("Press 'n' to start another playlist, or 'q' to quit."),
    ])
    .block(Block::default().title(" Done ").borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let help = match app.screen {
        Screen::UrlInput => "Type URL · Enter: fetch · Esc/Ctrl+C: quit",
        Screen::TrackList => {
            "↑/↓ move · Space toggle · a/n all/none · c disc size · [ ] target discs · 0 auto · d download · b burn · Esc back · Ctrl+C quit"
        }
        Screen::Working => {
            if app.awaiting_swap.is_some() {
                "Enter: continue to next disc · Ctrl+C: quit"
            } else {
                "Ctrl+C: quit"
            }
        }
        Screen::Done => "n: new playlist · q: quit",
    };
    let p = Paragraph::new(help).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
