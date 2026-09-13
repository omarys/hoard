use crate::core::trove::Trove;
use crate::core::{HoardCmd, string_to_tags};
use crate::gui::commands_gui::{DrawState, EditSelection, State};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn key_handler(input: KeyEvent, app: &mut State, default_namespace: &str) -> Option<HoardCmd> {
    // Make sure there is an empty command set
    if app.new_command.is_none() {
        app.new_command = Some(HoardCmd::default());
    }
    let ctrl = input.modifiers.contains(KeyModifiers::CONTROL);
    match input.code {
        KeyCode::Esc => {
            app.draw = DrawState::Search;
            app.new_command = None;
            app.edit_selection = EditSelection::Command;
            None
        }
        // Quit program
        KeyCode::Char('c' | 'd' | 'g') if ctrl => {
            app.should_exit = true;
            app.new_command = None;
            app.edit_selection = EditSelection::Command;
            None
        }
        KeyCode::Enter | KeyCode::Char('\r') => {
            let mut command = app.new_command.clone()?;
            let parameter = app.input.clone();
            app.error_message = match app.edit_selection {
                EditSelection::Command => {
                    command.command = parameter.clone();
                    match HoardCmd::is_command_valid(&parameter) {
                        Ok(()) => String::new(),
                        Err(error) => error.to_string(),
                    }
                }
                EditSelection::Name => {
                    command.name = parameter.clone();
                    let mut msg = match HoardCmd::is_name_valid(&parameter) {
                        Ok(()) => String::new(),
                        Err(error) => error.to_string(),
                    };
                    let trove = Trove::from_commands(&app.commands);
                    if trove.get_command_collision(&command).is_some() {
                        msg = String::from(
                            "Command with that name already exists in another namespace",
                        );
                    }
                    msg
                }
                EditSelection::Namespace => {
                    if parameter.is_empty() {
                        command.namespace = default_namespace.into();
                    } else {
                        command.namespace = parameter;
                    }
                    String::new()
                }
                EditSelection::Description => {
                    command.description = parameter;
                    String::new()
                }
                EditSelection::Tags => match HoardCmd::are_tags_valid(&parameter) {
                    Ok(()) => {
                        command.tags = string_to_tags(&parameter);
                        String::new()
                    }
                    Err(e) => {
                        app.error_message = e.to_string();
                        e.to_string()
                    }
                },
            };
            app.input = String::new();
            if !app.error_message.is_empty() {
                return None;
            }
            app.edit_selection = app.edit_selection.edit_next();
            if app.edit_selection == EditSelection::Command {
                return Some(command);
            }
            app.new_command = Some(command);
            None
        }
        // Handle query input
        KeyCode::Backspace => {
            app.input.pop();
            None
        }
        KeyCode::Char(c) => {
            app.input.push(c);
            None
        }
        _ => None,
    }
}
