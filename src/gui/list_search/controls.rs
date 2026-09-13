use crate::core::HoardCmd;
use crate::core::parameters::Parameterized;
use crate::gui::commands_gui::{ControlState, DrawState, EditSelection, State};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[allow(clippy::too_many_lines)]
pub fn key_handler(
    input: KeyEvent,
    state: &mut State,
    trove_commands: &[HoardCmd],
    namespace_tabs: &[&str],
) -> Option<HoardCmd> {
    let ctrl = input.modifiers.contains(KeyModifiers::CONTROL);
    match input.code {
        // Quit
        KeyCode::Esc | KeyCode::Char('c' | 'd' | 'g') if ctrl => {
            state.control = ControlState::Search;
            state.should_exit = true;
            None
        }
        KeyCode::Esc => {
            state.control = ControlState::Search;
            state.should_exit = true;
            None
        }
        // Show help
        KeyCode::F(1) => {
            state.draw = DrawState::Help;
            None
        }
        // Create new command
        KeyCode::Char('w') if ctrl => {
            state.draw = DrawState::Create;
            state.edit_selection = EditSelection::Command;
            state.new_command = Some(HoardCmd::default());
            None
        }
        // Enter GPT mode
        KeyCode::Char('a') if ctrl => {
            state.draw = DrawState::Search;
            if state.openai_key_set {
                state.control = ControlState::Gpt;
            } else {
                state.control = ControlState::KeyNotSet;
                state.query_gpt = true;
            }
            state.new_command = Some(HoardCmd::default());
            None
        }
        // Switch to edit mode
        KeyCode::Char('e') if ctrl => {
            let selected_command = state
                .commands
                .get(
                    state
                        .command_list
                        .selected()
                        .expect("there is always a selected command"),
                )
                .cloned()?;
            state.control = ControlState::Edit;
            state.selected_command = Some(selected_command);
            state.update_string_to_edit();
            None
        }
        KeyCode::Tab => {
            let selected_command = state
                .commands
                .get(
                    state
                        .command_list
                        .selected()
                        .expect("there is always a selected command"),
                )
                .cloned()?;
            state.control = ControlState::Edit;
            state.selected_command = Some(selected_command);
            state.update_string_to_edit();
            None
        }
        // Switch namespace
        KeyCode::Left => {
            if let Some(selected) = state.namespace_tab.selected() {
                let new_selected_tab = previous_index(selected, namespace_tabs.len());
                switch_namespace(state, new_selected_tab, namespace_tabs, trove_commands);
            }
            None
        }
        KeyCode::Char('h') if ctrl => {
            if let Some(selected) = state.namespace_tab.selected() {
                let new_selected_tab = previous_index(selected, namespace_tabs.len());
                switch_namespace(state, new_selected_tab, namespace_tabs, trove_commands);
            }
            None
        }
        KeyCode::Right => {
            if let Some(selected) = state.namespace_tab.selected() {
                let new_selected_tab = next_index(selected, namespace_tabs.len());
                switch_namespace(state, new_selected_tab, namespace_tabs, trove_commands);
            }
            None
        }
        KeyCode::Char('l') if ctrl => {
            if let Some(selected) = state.namespace_tab.selected() {
                let new_selected_tab = next_index(selected, namespace_tabs.len());
                switch_namespace(state, new_selected_tab, namespace_tabs, trove_commands);
            }
            None
        }
        // Switch command
        KeyCode::Up => {
            if !state.commands.is_empty()
                && let Some(selected) = state.command_list.selected()
            {
                let new_selected = previous_index(selected, state.commands.len());
                state.command_list.select(Some(new_selected));
            }
            None
        }
        KeyCode::Char('y' | 'p') if ctrl => {
            if !state.commands.is_empty()
                && let Some(selected) = state.command_list.selected()
            {
                let new_selected = previous_index(selected, state.commands.len());
                state.command_list.select(Some(new_selected));
            }
            None
        }
        KeyCode::Down => {
            if !state.commands.is_empty()
                && let Some(selected) = state.command_list.selected()
            {
                let new_selected = next_index(selected, state.commands.len());
                state.command_list.select(Some(new_selected));
            }
            None
        }
        KeyCode::Char('n' | '.') if ctrl => {
            if !state.commands.is_empty()
                && let Some(selected) = state.command_list.selected()
            {
                let new_selected = next_index(selected, state.commands.len());
                state.command_list.select(Some(new_selected));
            }
            None
        }
        // Delete
        KeyCode::Char('x') if ctrl => {
            if state.commands.is_empty() {
                return None;
            }
            let selected_command = state
                .commands
                .get(
                    state
                        .command_list
                        .selected()
                        .expect("there is always a selected command"),
                )
                .cloned();
            state.should_delete = true;
            selected_command
        }
        // Select command
        KeyCode::Enter | KeyCode::Char('\r' | '\n') => {
            if state.commands.is_empty() {
                state.should_exit = true;
                return None;
            }
            let selected_command = state
                .commands
                .get(
                    state
                        .command_list
                        .selected()
                        .expect("there is always a selected command"),
                )
                .cloned()?;
            // Check if parameters need to be supplied
            if selected_command.get_parameter_count(&state.parameter_token) > 0 {
                // Set next state to draw
                state.draw = DrawState::ParameterInput;
                // Save which command to replace parameters for
                state.selected_command = Some(selected_command);
                // Empty input for next screen
                state.input = String::new();
                // return None, otherwise drawing will quit
                return None;
            }
            Some(selected_command)
        }
        // Handle query input
        KeyCode::Backspace => {
            state.input.pop();
            apply_filter(state, namespace_tabs, trove_commands);
            None
        }
        KeyCode::Char(c) if !ctrl => {
            state.input.push(c);
            apply_filter(state, namespace_tabs, trove_commands);
            None
        }
        _ => None,
    }
}

const fn next_index(current_index: usize, collection_length: usize) -> usize {
    if current_index >= collection_length - 1 {
        0
    } else {
        current_index + 1
    }
}

const fn previous_index(current_index: usize, collection_length: usize) -> usize {
    if current_index > 0 {
        current_index - 1
    } else {
        collection_length - 1
    }
}

fn switch_namespace(
    state: &mut State,
    index_to_select: usize,
    namespaces: &[&str],
    commands: &[HoardCmd],
) {
    state.namespace_tab.select(Some(index_to_select));

    let selected_namespace = namespaces
        .get(index_to_select)
        .expect("Always a tab selected");

    apply_search(state, commands, selected_namespace);

    let new_selected_command = if state.commands.is_empty() {
        0
    } else {
        state.commands.len() - 1
    };

    state.command_list.select(Some(new_selected_command));
}

fn apply_search(state: &mut State, all_commands: &[HoardCmd], selected_tab: &str) {
    let query_term = &state.input[..];
    state.commands = all_commands
        .iter()
        .filter(|&c| {
            (c.name.contains(query_term)
                || c.namespace.contains(query_term)
                || c.get_tags_as_string().contains(query_term)
                || c.command.contains(query_term)
                || c.description.contains(query_term))
                && (c.namespace.clone() == *selected_tab || selected_tab == "All")
        })
        .cloned()
        .collect();
    state
        .commands
        .sort_by_key(|c| std::cmp::Reverse(c.usage_count));
}

fn apply_filter(state: &mut State, namespaces: &[&str], commands: &[HoardCmd]) {
    let selected_tab = namespaces
        .get(
            state
                .namespace_tab
                .selected()
                .expect("Always a namespace selected"),
        )
        .expect("Always a tab selected");
    apply_search(state, commands, selected_tab);
}

#[cfg(test)]
mod test_controls {
    use super::*;
    use ratatui::widgets::ListState;

    const DEFAULT_NAMESPACE: &str = "default";

    fn create_command(name: &str, command: &str, namespace: &str) -> HoardCmd {
        HoardCmd::default()
            .with_name(name)
            .with_command(command)
            .with_namespace(namespace)
    }

    fn create_state(commands: Vec<HoardCmd>) -> State {
        let mut state = State {
            input: String::new(),
            commands,
            command_list: ListState::default(),
            namespace_tab: ListState::default(),
            should_exit: false,
            should_delete: false,
            draw: DrawState::Search,
            control: ControlState::Search,
            new_command: None,
            edit_selection: crate::gui::commands_gui::EditSelection::Command,
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

        state.command_list.select(Some(0));
        state.namespace_tab.select(Some(0));

        state
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn test_change_command(key: KeyEvent, initial_index: usize, expected_index: usize) {
        let namespaces = vec![DEFAULT_NAMESPACE];
        let cmd1 = create_command("first", "", DEFAULT_NAMESPACE);
        let cmd2 = create_command("second", "", DEFAULT_NAMESPACE);
        let cmd3 = create_command("third", "", DEFAULT_NAMESPACE);
        let mut state = create_state(vec![cmd1, cmd2, cmd3]);
        state.command_list.select(Some(initial_index));

        let commands = state.commands.clone();
        key_handler(key, &mut state, &commands, &namespaces);
        let new_selected_index = state.command_list.selected();

        assert_eq!(expected_index, new_selected_index.unwrap());
    }

    fn test_change_namespace(key: KeyEvent, initial_index: usize, expected_index: usize) {
        let namespaces = vec!["first", "second", "third"];
        let mut state = create_state(vec![]);
        state.namespace_tab.select(Some(initial_index));

        let commands = state.commands.clone();
        key_handler(key, &mut state, &commands, &namespaces);
        let new_selected_index = state.namespace_tab.selected();

        assert_eq!(expected_index, new_selected_index.unwrap());
    }

    // Commands
    #[test]
    fn next_command() {
        test_change_command(key(KeyCode::Down), 0, 1);
    }

    #[test]
    fn next_command_wrap() {
        test_change_command(key(KeyCode::Down), 2, 0);
    }

    #[test]
    fn previous_command() {
        test_change_command(key(KeyCode::Up), 2, 1);
    }

    #[test]
    fn previous_command_wrap() {
        test_change_command(key(KeyCode::Up), 0, 2);
    }

    // Namespaces
    #[test]
    fn next_namespace() {
        test_change_namespace(key(KeyCode::Right), 1, 2);
    }

    #[test]
    fn next_namespace_wrap() {
        test_change_namespace(key(KeyCode::Right), 2, 0);
    }

    #[test]
    fn previous_namespace() {
        test_change_namespace(key(KeyCode::Left), 2, 1);
    }

    #[test]
    fn previous_namespace_wrap() {
        test_change_namespace(key(KeyCode::Left), 0, 2);
    }

    #[test]
    fn filter_commands_when_namespace_changed() {
        let namespace1 = "first_namespace";
        let namespace2 = "second_namespace";
        let all_namespaces = vec![namespace1, namespace2];

        let cmd2_name = "second_command";
        let cmd1 = create_command("first_command", "", namespace1);
        let cmd2 = create_command(cmd2_name, "", namespace2);
        let mut state = create_state(vec![cmd1, cmd2]);

        let commands = state.commands.clone();
        key_handler(key(KeyCode::Right), &mut state, &commands, &all_namespaces);
        let filtered_commands = state.commands;

        assert_eq!(1, filtered_commands.len());
        assert_eq!(cmd2_name, filtered_commands.first().unwrap().name);
    }

    #[test]
    fn select_last_command_when_namespace_changed() {
        let namespace1 = "first_namespace";
        let namespace2 = "second_namespace";
        let all_namespaces = vec![namespace1, namespace2];

        let expected_command_index = 1;
        let cmd1 = create_command("first_command", "", namespace2);
        let cmd2 = create_command("second_command", "", namespace2);
        let mut state = create_state(vec![cmd1, cmd2]);

        let commands = state.commands.clone();
        key_handler(key(KeyCode::Right), &mut state, &commands, &all_namespaces);
        let selected_command_index = state.command_list.selected().unwrap();

        assert_eq!(expected_command_index, selected_command_index);
    }

    #[test]
    fn pick_command_without_params() {
        let namespaces = vec![DEFAULT_NAMESPACE];
        let expected_command = "second_command";
        let command_index = 1;
        let cmd1 = create_command("first_command", "", DEFAULT_NAMESPACE);
        let cmd2 = create_command(expected_command, "", DEFAULT_NAMESPACE);

        let mut state = create_state(vec![cmd1, cmd2]);
        state.command_list.select(Some(command_index));

        let commands = state.commands.clone();
        let actual_command = key_handler(key(KeyCode::Enter), &mut state, &commands, &namespaces)
            .expect("a command should be selected");

        assert_eq!(expected_command, actual_command.name);
    }

    #[test]
    fn pick_command_with_params() {
        let namespaces = vec![DEFAULT_NAMESPACE];
        let cmd = create_command("First", "first_command #", DEFAULT_NAMESPACE);

        let mut state = create_state(vec![cmd]);
        let commands = state.commands.clone();
        key_handler(key(KeyCode::Enter), &mut state, &commands, &namespaces);

        assert_eq!(DrawState::ParameterInput, state.draw);
    }

    #[test]
    fn quit_on_nothing_to_pick() {
        let mut state = create_state(vec![]);

        key_handler(key(KeyCode::Enter), &mut state, &[], &[]);

        assert!(state.should_exit);
    }

    #[test]
    fn quit() {
        let mut state = create_state(vec![]);

        key_handler(key(KeyCode::Esc), &mut state, &[], &[]);

        assert!(state.should_exit);
    }

    #[test]
    fn quit_with_ctrl_c() {
        let mut state = create_state(vec![]);

        key_handler(ctrl('c'), &mut state, &[], &[]);

        assert!(state.should_exit);
    }

    #[test]
    fn show_help() {
        let mut state = create_state(vec![]);

        key_handler(key(KeyCode::F(1)), &mut state, &[], &[]);

        assert_eq!(DrawState::Help, state.draw);
    }
}
