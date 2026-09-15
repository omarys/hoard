use crate::config::HoardConfig;
use crate::core::{CommandKind, HoardCmd};
use crate::gui::commands_gui::State;
use crate::gui::commands_gui::{ControlState, EditSelection};
use crate::gui::help::HELP_KEY;
use crate::theme::BACKGROUND;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Terminal width (columns) at and above which the command list and the detail
/// pane are laid out side by side. Below it they stack vertically so the list
/// keeps the full width.
const SPLIT_MIN_WIDTH: u16 = 110;
/// Share of the available width given to the command list in split mode.
const LIST_SPLIT_SHARE: u16 = 40;

/// How the main area (list + detail) is arranged for a given terminal width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutMode {
    /// List on top, detail pane below (narrow terminals).
    Stacked,
    /// List on the left, detail pane on the right (wide terminals).
    Split,
}

/// Picks the layout for the current terminal width.
const fn layout_mode(width: u16) -> LayoutMode {
    if width >= SPLIT_MIN_WIDTH {
        LayoutMode::Split
    } else {
        LayoutMode::Stacked
    }
}

/// Dracula background as a reusable style.
fn bg() -> Style {
    Style::default().bg(Color::Rgb(BACKGROUND.0, BACKGROUND.1, BACKGROUND.2))
}

fn rgb(color: (u8, u8, u8)) -> Color {
    Color::Rgb(color.0, color.1, color.2)
}

/// Primary (foreground) color from the config on the Dracula background.
fn primary_style(config: &HoardConfig) -> Style {
    bg().fg(rgb(config.primary_color.unwrap()))
}

#[allow(clippy::too_many_lines)]
pub fn draw(
    app_state: &mut State,
    config: &HoardConfig,
    namespace_tabs: &[&str],
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), eyre::Error> {
    terminal.draw(|rect| {
        rect.render_widget(Block::default().style(bg()), rect.area());
        let size = rect.area();
        let chunks = Layout::vertical([
            Constraint::Length(3), // namespace tabs
            Constraint::Min(2),    // list + detail
            Constraint::Length(3), // query input
            Constraint::Length(1), // footer
        ])
        .margin(1)
        .split(size);

        rect.render_widget(tabs_widget(app_state, config, namespace_tabs), chunks[0]);

        // Responsive main area: split side by side on wide terminals, stacked
        // on narrow ones.
        let (list_area, detail_area) = match layout_mode(size.width) {
            LayoutMode::Split => {
                let chunks = Layout::horizontal([
                    Constraint::Percentage(LIST_SPLIT_SHARE),
                    Constraint::Percentage(100 - LIST_SPLIT_SHARE),
                ])
                .split(chunks[1]);
                (chunks[0], chunks[1])
            }
            LayoutMode::Stacked => {
                let chunks =
                    Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)])
                        .split(chunks[1]);
                (chunks[0], chunks[1])
            }
        };

        rect.render_stateful_widget(
            commands_widget(app_state, config),
            list_area,
            &mut app_state.command_list,
        );

        let (tags, description, command) = detail_widgets(app_state, config);
        let is_python = app_state
            .commands
            .get(app_state.command_list.selected().unwrap_or(0))
            .is_some_and(|command| command.kind == CommandKind::Python);
        let detail_chunks = Layout::vertical([
            Constraint::Length(3), // tags
            Constraint::Min(2),    // description
            if is_python {
                Constraint::Percentage(60)
            } else {
                Constraint::Length(3)
            },
        ])
        .split(detail_area);
        rect.render_widget(tags, detail_chunks[0]);
        rect.render_widget(description, detail_chunks[1]);
        rect.render_widget(command, detail_chunks[2]);

        rect.render_widget(input_widget(app_state, config), chunks[2]);

        let (control_hint, shortcuts_hint) = footer_widgets(&app_state.control, config);
        let footer_chunk =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[3]);
        rect.render_widget(control_hint, footer_chunk[0]);
        if app_state.control == ControlState::Search {
            rect.render_widget(shortcuts_hint, footer_chunk[1]);
        }

        if app_state.query_gpt {
            let msg = if app_state.openai_key_set {
                State::get_default_popupmsg()
            } else {
                State::get_no_api_key_popupmsg()
            };
            let popup = Paragraph::new(msg)
                .style(primary_style(config))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(bg().fg(get_color(app_state, config, &EditSelection::Description)))
                        .title("GPT")
                        .border_type(BorderType::Plain),
                );
            let area = centered_rect(50, 10, size);
            rect.render_widget(Clear, area); // clears out the background
            rect.render_widget(popup, area);
        }
    })?;
    Ok(())
}

/// The namespace tabs row.
fn tabs_widget<'a>(
    app_state: &'a State,
    config: &'a HoardConfig,
    namespace_tabs: &'a [&'a str],
) -> Tabs<'a> {
    let menu: Vec<Line> = namespace_tabs
        .iter()
        .map(|t| Line::from(vec![Span::styled(*t, primary_style(config))]))
        .collect();

    Tabs::new(menu)
        .select(
            app_state
                .namespace_tab
                .selected()
                .expect("Always a namespace selected"),
        )
        .block(
            Block::default()
                .title(" Hoard Namespace ")
                .borders(Borders::ALL),
        )
        .style(primary_style(config))
        .highlight_style(
            Style::default()
                .fg(rgb(config.secondary_color.unwrap()))
                .add_modifier(Modifier::UNDERLINED),
        )
        .divider(Span::raw("|"))
}

/// The selectable command list, keeping the selection in bounds.
fn commands_widget<'a>(app: &mut State, config: &HoardConfig) -> List<'a> {
    let block = Block::default()
        .borders(Borders::ALL)
        .style(bg().fg(get_color(app, config, &EditSelection::Name)))
        .title(" Commands ")
        .border_type(BorderType::Plain);

    let items: Vec<_> = app
        .commands
        .iter()
        .map(|command| {
            let name = Span::styled(command.name.clone(), Style::default());
            let usage = Span::styled(
                format!(" · {}×", command.usage_count),
                Style::default().fg(rgb(crate::theme::COMMENT)),
            );
            ListItem::new(Line::from(vec![name, usage]))
        })
        .collect();

    // Keep the selection in bounds if it drifted past the last command.
    if let Some(selected) = app.command_list.selected()
        && selected >= app.commands.len()
    {
        app.command_list
            .select(Some(app.commands.len().saturating_sub(1)));
    }

    List::new(items).block(block).highlight_style(
        Style::default()
            .bg(rgb(config.secondary_color.unwrap()))
            .fg(rgb(config.tertiary_color.unwrap()))
            .add_modifier(Modifier::BOLD),
    )
}

/// The selected command's tags, description and command text.
fn detail_widgets<'a>(
    app: &State,
    config: &HoardConfig,
) -> (Paragraph<'a>, Paragraph<'a>, Paragraph<'a>) {
    let selected = app
        .commands
        .get(app.command_list.selected().unwrap_or(0))
        .cloned()
        .unwrap_or_else(HoardCmd::default);

    let tags = Paragraph::new(coerce_string_by_mode(
        selected.get_tags_as_string(),
        app,
        &EditSelection::Tags,
    ))
    .style(primary_style(config))
    .alignment(Alignment::Left)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(bg().fg(get_color(app, config, &EditSelection::Tags)))
            .title(" Tags ")
            .border_type(BorderType::Plain),
    );

    let description = Paragraph::new(coerce_string_by_mode(
        selected.description,
        app,
        &EditSelection::Description,
    ))
    .style(primary_style(config))
    .alignment(Alignment::Left)
    .wrap(Wrap { trim: true })
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(bg().fg(get_color(app, config, &EditSelection::Description)))
            .title(" Description ")
            .border_type(BorderType::Plain),
    );

    let title = format!(
        " {} | Times selected: {} ",
        if selected.kind == CommandKind::Python {
            "Python script"
        } else {
            "Hoarded command"
        },
        selected.usage_count
    );
    let command = Paragraph::new(coerce_string_by_mode(
        selected.command,
        app,
        &EditSelection::Command,
    ))
    .style(primary_style(config))
    .alignment(Alignment::Left)
    .wrap(Wrap {
        trim: selected.kind != CommandKind::Python,
    })
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(bg().fg(get_color(app, config, &EditSelection::Command)))
            .title(title)
            .border_type(BorderType::Plain),
    );

    (tags, description, command)
}

/// The query input line.
fn input_widget<'a>(app: &State, config: &HoardConfig) -> Paragraph<'a> {
    let mut query_string = config.query_prefix.clone();
    query_string.push_str(&app.input);
    Paragraph::new(query_string).block(
        Block::default()
            .style(primary_style(config))
            .borders(Borders::ALL)
            .title(format!(" hoard v{VERSION} ")),
    )
}

/// The footer: current control mode on the left, shortcut hints on the right.
fn footer_widgets<'a>(
    control: &ControlState,
    config: &HoardConfig,
) -> (Paragraph<'a>, Paragraph<'a>) {
    let control_hint = Paragraph::new(format!("{control}"))
        .style(primary_style(config))
        .alignment(Alignment::Left);
    let shortcuts_hint = Paragraph::new(format!(
        "Create <Ctrl-W> | Delete <Ctrl-X> | GPT <Ctrl-A> | Help {HELP_KEY}"
    ))
    .style(primary_style(config))
    .alignment(Alignment::Right);
    (control_hint, shortcuts_hint)
}

/// Helper to create a centered rect using up certain percentage of the
/// available rect `r`.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

/// The color for panel borders: the highlighted (secondary) one for the field
/// being edited, the primary one otherwise.
fn get_color(app: &State, config: &HoardConfig, command_render: &EditSelection) -> Color {
    let highlighted = rgb(config.secondary_color.unwrap());
    let normal = rgb(config.primary_color.unwrap());
    if app.control == ControlState::Edit && command_render == &app.edit_selection {
        highlighted
    } else {
        normal
    }
}

/// Shows the edit buffer instead of the stored value for a field being edited.
fn coerce_string_by_mode(s: String, app: &State, command_render: &EditSelection) -> String {
    if app.control == ControlState::Edit && command_render == &app.edit_selection {
        app.string_to_edit.clone()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_screens_stack() {
        assert_eq!(layout_mode(80), LayoutMode::Stacked);
    }

    #[test]
    fn wide_screens_split() {
        assert_eq!(layout_mode(200), LayoutMode::Split);
    }

    #[test]
    fn split_threshold() {
        assert_eq!(layout_mode(SPLIT_MIN_WIDTH), LayoutMode::Split);
        assert_eq!(layout_mode(SPLIT_MIN_WIDTH - 1), LayoutMode::Stacked);
    }

    #[test]
    fn colors_follow_config() {
        let config = HoardConfig::default();
        let is_editing = State {
            input: String::new(),
            commands: vec![],
            command_list: ratatui::widgets::ListState::default(),
            namespace_tab: ratatui::widgets::ListState::default(),
            should_exit: false,
            should_delete: false,
            draw: crate::gui::commands_gui::DrawState::Search,
            control: ControlState::Edit,
            edit_selection: EditSelection::Name,
            new_command: None,
            string_to_edit: String::new(),
            parameter_token: "#".to_string(),
            parameter_ending_token: "!".to_string(),
            selected_command: None,
            provided_parameter_count: 0,
            error_message: String::new(),
            query_gpt: false,
            buffered_tick: false,
            openai_key_set: false,
        };
        assert_eq!(
            get_color(&is_editing, &config, &EditSelection::Name),
            rgb(config.secondary_color.unwrap())
        );
    }
}
