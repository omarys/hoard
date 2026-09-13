use crate::config::HoardConfig;
use crate::core::HoardCmd;
use crate::gui::commands_gui::{DrawState, State};
use crate::theme::BACKGROUND;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem};

pub const HELP_KEY: &str = "<F1>";

/// Dracula background as a reusable style.
fn bg() -> Style {
    Style::default().bg(Color::Rgb(BACKGROUND.0, BACKGROUND.1, BACKGROUND.2))
}
const HELP_CONTENT: &[(&str, &str)] = &[
    ("Next item in command list", "<Ctrl-N> / <Down-Arrow>"),
    (
        "Previous item in command list",
        "<Ctrl-P> / <Ctrl-Y> / <Up-Arrow>",
    ),
    ("Next namespace tab", "<Ctrl-L> / <Right-Arrow>"),
    ("Previous namespace tab", "<Ctrl-H> / <Left-Arrow>"),
    ("Select command", "<Enter>"),
    ("Create new command", "<Ctrl-W>"),
    ("Delete command", "<Ctrl-X>"),
    ("Toggle search/edit mode", "<Tab> / <Ctrl-E>"),
    ("Toggle Command to edit in edit mode", "<Tab>"),
    ("Exit edit mode", "<Esc>"),
    ("Quit", "<Ctrl-D> / <Ctrl-C> / <Ctrl-G>"),
    ("Show help", HELP_KEY),
    ("Close help", "<Any key>"),
];

pub fn draw(
    config: &HoardConfig,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), eyre::Error> {
    terminal.draw(|rect| {
        rect.render_widget(Block::default().style(bg()), rect.area());
        let help = Block::default()
            .borders(Borders::ALL)
            .style(bg().fg(Color::Rgb(
                config.primary_color.unwrap().0,
                config.primary_color.unwrap().1,
                config.primary_color.unwrap().2,
            )))
            .title(" Help ")
            .border_type(BorderType::Plain);

        let items: Vec<_> = HELP_CONTENT
            .iter()
            .map(|item| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        item.0,
                        Style::default().fg(Color::Rgb(
                            config.command_color.unwrap().0,
                            config.command_color.unwrap().1,
                            config.command_color.unwrap().2,
                        )),
                    )),
                    Line::from(Span::styled(
                        format!("    {}", item.1),
                        Style::default().fg(Color::Rgb(
                            config.primary_color.unwrap().0,
                            config.primary_color.unwrap().1,
                            config.primary_color.unwrap().2,
                        )),
                    )),
                    Line::from(""),
                ])
            })
            .collect();

        let list = List::new(items).block(help);
        rect.render_widget(list, rect.area());
    })?;
    Ok(())
}

pub fn key_handler(_input: crossterm::event::KeyEvent, app: &mut State) -> Option<HoardCmd> {
    app.draw = DrawState::Search;
    None
}
