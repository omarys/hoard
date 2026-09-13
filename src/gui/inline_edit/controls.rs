use crate::core::{HoardCmd, string_to_tags};
use crate::gui::commands_gui::{ControlState, EditSelection, State};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn key_handler(input: KeyEvent, state: &mut State) -> Option<HoardCmd> {
    let ctrl = input.modifiers.contains(KeyModifiers::CONTROL);
    match input.code {
        // Quit edit mode
        KeyCode::Esc => {
            state.control = ControlState::Search;
            None
        }
        // Confirm edit
        KeyCode::Enter | KeyCode::Char('\r') => {
            let mut edited_command = state.selected_command.clone()?;
            let new_string = state.string_to_edit.clone();
            match state.edit_selection {
                EditSelection::Description => edited_command.description = new_string,
                EditSelection::Command => edited_command.command = new_string,
                EditSelection::Tags => edited_command.tags = string_to_tags(&new_string),
                EditSelection::Name | EditSelection::Namespace => (),
            };
            Some(edited_command)
        }
        // Switch field to edit
        KeyCode::Tab => {
            state.edit_selection = state.edit_selection.next();
            state.update_string_to_edit();
            None
        }
        // Exit program
        KeyCode::Char('c' | 'd' | 'g') if ctrl => {
            state.should_exit = true;
            None
        }
        // Handle query input
        KeyCode::Backspace => {
            state.string_to_edit.pop();
            None
        }
        KeyCode::Char(c) => {
            state.string_to_edit.push(c);
            None
        }
        _ => None,
    }
}
