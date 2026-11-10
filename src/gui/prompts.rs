use crate::gui::theme::HoardTheme;
use dialoguer::{Input, MultiSelect, Select};
pub enum Confirmation {
    Yes,
    No,
}

pub fn prompt_multiselect_options<F, T, S>(
    question: &str,
    selection_prompt: &str,
    options: &[T],
    text_extractor: F,
) -> Vec<T>
where
    F: FnMut(&T) -> S,
    T: Clone,
    S: ToString,
{
    let options_texts: Vec<S> = options.iter().map(text_extractor).collect();

    if matches!(prompt_yes_or_no(question), Confirmation::Yes) {
        match MultiSelect::with_theme(&HoardTheme::default())
            .with_prompt(selection_prompt)
            .items(options_texts)
            .interact()
        {
            Ok(selected_indices) => take_elements_by_indices(options, &selected_indices),
            Err(err) => {
                warn_no_console("selection", &err);
                Vec::new()
            }
        }
    } else {
        options.to_vec()
    }
}

pub fn prompt_yes_or_no(text: &str) -> Confirmation {
    const YES_ANSWER: usize = 0;

    let answer = Select::with_theme(&HoardTheme::default())
        .with_prompt(text)
        .items(["Yes", "No"])
        .default(YES_ANSWER)
        .interact()
        .unwrap_or_else(|err| {
            warn_no_console("question", &err);
            // "No" is the answer that changes nothing.
            1
        });

    if answer == YES_ANSWER {
        Confirmation::Yes
    } else {
        Confirmation::No
    }
}

pub fn prompt_select_with_options(text_prompt: &str, options: &[&str]) -> usize {
    Select::with_theme(&HoardTheme::default())
        .with_prompt(text_prompt)
        .items(options.iter().copied())
        .default(0)
        .interact()
        .unwrap_or_else(|err| {
            warn_no_console("choice", &err);
            0
        })
}

pub fn prompt_input(text: &str, allow_empty: bool, default_value: Option<String>) -> String {
    // Just calls `prompt_input_validate` to not keep on typing `None` for the validator
    prompt_input_validate(
        text,
        allow_empty,
        default_value,
        None::<Box<dyn FnMut(&String) -> Result<(), String>>>,
    )
}

pub fn prompt_input_validate<F>(
    text: &str,
    allow_empty: bool,
    default_value: Option<String>,
    validator: Option<F>,
) -> String
where
    F: FnMut(&String) -> Result<(), String>,
{
    let theme = HoardTheme::default();
    let mut input = Input::with_theme(&theme).with_prompt(text);
    let mut fallback = String::new();
    // Add default value to input prompt
    if let Some(val) = default_value {
        fallback = val.clone();
        input = input.default(val);
    }
    // Add validator if any
    if let Some(val) = validator {
        input = input.validate_with(val);
    }
    input
        .allow_empty(allow_empty)
        .interact_text()
        .unwrap_or_else(|err| {
            warn_no_console("input", &err);
            fallback
        })
}

/// Report a prompt that could not run, e.g. when stdin is not a terminal.
///
/// Prompting is impossible without a console, so callers fall back to the
/// answer that changes nothing instead of panicking.
fn warn_no_console(what: &str, err: &dialoguer::Error) {
    eprintln!("cannot ask for {what} ({err}); proceeding with the safe default");
}

fn take_elements_by_indices<T>(elements: &[T], indices: &[usize]) -> Vec<T>
where
    T: Clone,
{
    elements
        .iter()
        .enumerate()
        .filter_map(|(i, val)| {
            if indices.contains(&i) {
                Some(val.clone())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod test_prompts {
    use super::*;

    #[test]
    fn elements_by_indices() {
        let items = vec!["item1", "item2", "item3", "item4"];
        let indices = vec![1, 3];
        let expected_items = vec![items[1], items[3]];

        assert_eq!(expected_items, take_elements_by_indices(&items, &indices));
    }
}
