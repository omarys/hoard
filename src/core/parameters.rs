use regex::Regex;

use crate::core::HoardCmd;
use crate::gui::prompts::prompt_input;

/// Default token marking the start of a named parameter in a command string.
pub const DEFAULT_PARAMETER_TOKEN: &str = "#";
/// Default token marking the end of a named parameter in a command string.
pub const DEFAULT_ENDING_PARAMETER_TOKEN: &str = "!";

/// Commands whose string may contain named parameters.
pub trait Parameterized {
    /// Counts how many times `token` appears in the command string.
    fn get_parameter_count(&self, token: &str) -> usize;

    /// Replaces a parameter identified by `start_token` .. `end_token` with
    /// `value`, returning a new command.
    fn replace_parameter(&self, start_token: &str, end_token: &str, value: &str) -> HoardCmd;

    /// Replaces every parameter in the command string with user input,
    /// prompting once per parameter, and returns the filled-in command.
    fn with_input_parameters(&mut self, token: &str, ending_token: &str) -> HoardCmd;
}

impl Parameterized for HoardCmd {
    fn get_parameter_count(&self, token: &str) -> usize {
        self.command.matches(token).count()
    }

    fn replace_parameter(&self, start_token: &str, end_token: &str, value: &str) -> Self {
        let pattern = format!(
            "{}.*?{}",
            regex::escape(start_token),
            regex::escape(end_token)
        );
        let re = Regex::new(&pattern).expect("parameter pattern is always valid");
        let replaced = re.replace_all(&self.command, value);
        Self::default().with_command(&replaced)
    }

    fn with_input_parameters(&mut self, token: &str, ending_token: &str) -> Self {
        let mut param_count = 0;
        while self.get_parameter_count(token) != 0 {
            let prompt_dialog = format!(
                "Enter parameter({token}) nr {} \n~> {}\n",
                param_count + 1,
                self.command
            );
            let parameter = prompt_input(&prompt_dialog, false, None);
            self.command = self
                .replace_parameter(token, ending_token, &parameter)
                .command;
            param_count += 1;
        }
        self.clone()
    }
}

#[cfg(test)]
mod test_commands {
    use super::*;

    #[test]
    fn test_get_parameter_count() {
        let command = HoardCmd::default().with_command("test test test");
        assert_eq!(3, command.get_parameter_count("test"));
    }

    #[test]
    fn test_replace_parameter() {
        let command = HoardCmd::default().with_command("test1 # test3");
        let expected = HoardCmd::default().with_command("test1 replacement test3");
        assert_eq!(expected, command.replace_parameter("#", "", "replacement"));
    }

    #[test]
    fn test_replace_parameter_with_endtoken() {
        let command = HoardCmd::default().with_command("test1 #thisisacommand! test3");
        let expected = HoardCmd::default().with_command("test1 replacement test3");
        assert_eq!(expected, command.replace_parameter("#", "!", "replacement"));
    }

    #[test]
    fn test_replace_parameter_with_endtoken_no_spaces() {
        let command = HoardCmd::default().with_command("test1#thisisacommand!test3");
        let expected = HoardCmd::default().with_command("test1replacementtest3");
        assert_eq!(expected, command.replace_parameter("#", "!", "replacement"));
    }
}
