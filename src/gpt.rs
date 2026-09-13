use crate::core::{HoardCmd, string_to_tags};
use serde::{Deserialize, Serialize};

const API_URL: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-3.5-turbo";

#[derive(Serialize)]
struct ChatRequest {
    model: &'static str,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

/// Parses a (admittedly loosely specified) GPT response into a command.
pub fn from_gpt_string(gpt_string: &str) -> HoardCmd {
    let mut cmd = HoardCmd::default();
    let mut name = "Something_went_wrong".to_owned();
    let mut tags = String::new();
    // If something goes wrong, we'll just use this as the description so the
    // user sees what's going on.
    let mut description =
        format!("Something went wrong when parsing the GPT response.\n{gpt_string}");
    let mut command = String::new();
    for line in gpt_string.lines() {
        for prefix in [
            "name: > ",
            "name: ",
            "explanation: > ",
            "explanation: ",
            "tags: > ",
            "tags: ",
            "command: > ",
            "command: ",
        ] {
            let Some(rest) = line.strip_prefix(prefix) else {
                continue;
            };
            match prefix {
                "name: > " | "name: " => name = rest.to_owned(),
                "explanation: > " | "explanation: " => description = rest.to_owned(),
                "tags: > " | "tags: " => tags = rest.replace(' ', ""),
                "command: > " | "command: " => command = rest.to_owned(),
                _ => unreachable!(),
            }
            break;
        }
    }
    cmd.name = name;
    cmd.description = description;
    cmd.command = command.clone();
    cmd.tags = string_to_tags(&tags);
    cmd.namespace = "gpt".to_string();
    if command.is_empty() {
        cmd.description = format!(
            "{}\n\nSomething probably went wrong parsing the GPT response:\n{gpt_string}",
            cmd.description
        );
    }
    cmd
}

/// Builds a command that carries the given problem in its description, so the
/// GUI can surface it instead of crashing.
fn error_command(message: &str) -> HoardCmd {
    HoardCmd {
        name: "Something_went_wrong".to_string(),
        description: message.to_string(),
        ..HoardCmd::default()
    }
}

/// Asks ChatGPT to create a command for `input` and returns the parsed result.
pub fn prompt(input: &str, key: &str) -> HoardCmd {
    let formatted_command = format!(
        "
Write a linux command that does the following:
{input}

Reply with a made up name for the command ( Example: '> find_and_replace' ).
Also a very short explanation without any formatting. ( Example: '> Does this thing')
After the the explanation add the command. ( Example: '> mv #file! #target!')
Come up with up to 3 tags for the command ( Example: '> git,filesystem' )
If there are any parameters, enclose them with #parameter_name!
This is the format how to reply:

name:<command name>

explanation:<short explanation>

tags: <tags>

command: <command>
    "
    );

    let request = ChatRequest {
        model: MODEL,
        messages: vec![Message {
            role: "user",
            content: formatted_command,
        }],
    };
    let client = reqwest::blocking::Client::new();
    match client.post(API_URL).bearer_auth(key).json(&request).send() {
        Ok(response) => match response.json::<ChatResponse>() {
            Ok(parsed) => from_gpt_string(&parsed.choices[0].message.content),
            Err(err) => error_command(&format!("Could not parse ChatGPT response: {err}")),
        },
        Err(err) => error_command(&format!("ChatGPT request failed: {err}")),
    }
}
