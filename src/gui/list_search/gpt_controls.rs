use crate::core::HoardCmd;
use crate::gui::commands_gui::{ControlState, DrawState, State};
use crossterm::event::{KeyCode, KeyEvent};

pub fn key_handler(input: KeyEvent, state: &mut State) -> Option<HoardCmd> {
    match input.code {
        KeyCode::Esc => {
            // Back to search
            state.control = ControlState::Search;
            state.query_gpt = false;
            None
        }
        // Show help
        KeyCode::F(1) => {
            state.draw = DrawState::Help;
            None
        }
        // Send query to GPT
        KeyCode::Enter | KeyCode::Char('\r') => {
            state.query_gpt = true;
            None
        }
        // Handle query input
        KeyCode::Backspace => {
            state.input.pop();
            None
        }
        KeyCode::Char(c) => {
            state.input.push(c);
            None
        }
        _ => None,
    }
}
