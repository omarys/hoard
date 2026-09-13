use crate::core::HoardCmd;
use crate::gui::commands_gui::{ControlState, State};
use crossterm::event::{KeyCode, KeyEvent};

pub fn key_handler(input: KeyEvent, state: &mut State) -> Option<HoardCmd> {
    match input.code {
        KeyCode::Esc => {
            // Back to search
            state.control = ControlState::Search;
            state.query_gpt = false;
            None
        }
        _ => None,
    }
}
