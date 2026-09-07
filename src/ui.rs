use crate::app::{App, Screen, TrackStatus};
use crate::burn;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
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
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let checkbox = if t.selected { "[x]" } else { "[ ]" };
            let status = match &t.status {
                TrackStatus::Pending => String::new(),
                TrackStatus::Downloading => " (downloading...)".to_string(),
                TrackStatus::Downloaded(_) => " (downloaded)".to_string(),
                TrackStatus::Failed(reason) => format!(" (FAILED: {})", truncate(reason, 40)),
            };
            let line = format!(
                "{checkbox} {:>3}  {:<50} {:>6}{status}",
                i + 1,
                truncate(&t.track.title, 50),
                t.track.duration_label()
            );
            let style = if i == app.cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                match t.status {
                    TrackStatus::Failed(_) => Style::default().fg(Color::Red),
                    TrackStatus::Downloaded(_) => Style::default().fg(Color::Green),
                    _ => Style::default(),
                }
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title(" Tracks ").borders(Borders::ALL));
    f.render_widget(list, chunks[0]);

    let total = app.total_selected_seconds();
    let selected_n = app.tracks.iter().filter(|t| t.selected).count();
    let fits = if burn::fits_on_one_cd(total) { "fits" } else { "OVER 80-min CD-R limit!" };
    let summary = format!(
        "{selected_n}/{} tracks selected · {}:{:02} total · {fits}",
        app.tracks.len(),
        total / 60,
        total % 60
    );
    let color = if burn::fits_on_one_cd(total) { Color::Green } else { Color::Red };
    let p = Paragraph::new(Span::styled(summary, Style::default().fg(color)))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p, chunks[1]);
}

fn draw_log(f: &mut Frame, area: Rect, app: &App) {
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
        Line::from(Span::styled("Disc burned. Go play it in the car.", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
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
            "↑/↓: move · Space: toggle · a: select all · n: select none · d: download selected · b: burn downloaded · Esc: back · Ctrl+C: quit"
        }
        Screen::Working => "Ctrl+C: quit",
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
