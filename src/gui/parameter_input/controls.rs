use crate::core::HoardCmd;
use crate::core::parameters::Parameterized;
use crate::gui::commands_gui::State;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn key_handler(input: KeyEvent, app: &mut State) -> Option<HoardCmd> {
    let ctrl = input.modifiers.contains(KeyModifiers::CONTROL);
    match input.code {
        // Quit program
        KeyCode::Esc | KeyCode::Char('c' | 'd' | 'g') if ctrl => {
            app.should_exit = true;
            None
        }
        KeyCode::Esc => {
            app.should_exit = true;
            None
        }
        // Confirm parameter for the next one
        KeyCode::Enter | KeyCode::Char('\r') => {
            let command = app.selected_command.clone()?;
            let parameter = app.input.clone();
            let replaced_command = command.replace_parameter(
                &app.parameter_token,
                &app.parameter_ending_token,
                &parameter,
            );
            app.input = String::new();
            if replaced_command.get_parameter_count(&app.parameter_token) == 0 {
                return Some(replaced_command);
            }
            app.selected_command = Some(replaced_command);
            app.provided_parameter_count += 1;
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
