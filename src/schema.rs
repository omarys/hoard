//! `hoard schema` — the machine-readable contract agents build calls from.
//!
//! This is a hand-written, deliberately small JSON Schema: it documents the
//! shapes `--json`/`--compact` emit, the exit codes, the placeholder syntax and
//! the arguments of every headless invocation. A model that reads it once can
//! construct correct calls without guessing flag names.

use crate::headless::{EXIT_INVALID_PARAM, EXIT_NOT_FOUND, EXIT_OK, EXIT_PARSE, EXIT_RUNTIME};
use serde_json::{Map, Value, json};

/// The contract document.
pub fn document() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/Hyde46/hoard/schema/v2",
        "title": "hoard headless contract",
        "description": "Command store for agents. Every payload is JSON on stdout when --json or --compact is given; failures always use the error shape and a fixed exit code.",
        "x-hoard-version": env!("CARGO_PKG_VERSION"),
        "exit_codes": exit_codes(),
        "placeholders": {
            "syntax": "{{name}} required, {{name=default}} optional with a default",
            "identifiers": "start with an ASCII letter or '_', then ASCII letters, digits, '_', '-', '.'",
            "literal": "any other {{...}} text is left untouched",
            "substitution": "values are inserted verbatim; quote them in the command, not on the CLI",
        },
        "invocations": {
            "search": {
                "argv": ["hoard", "list", "--json", "--compact", "-f", "<query>"],
                "returns": "#/$defs/compactList",
            },
            "best": {
                "argv": ["hoard", "list", "--json", "--compact", "--best", "-f", "<query>"],
                "returns": "#/$defs/compactCommand",
                "note": "exits with CommandNotFound when nothing matches",
            },
            "eval": {
                "argv": ["hoard", "eval", "<name>", "-p", "<key>=<value>", "--json", "--compact"],
                "arguments": {
                    "name": "positional: entry name, or namespace/name when the name is ambiguous",
                    "param": "repeatable -p/--param KEY=VALUE; unknown keys or missing required keys fail with InvalidParameter",
                },
                "returns": "#/$defs/evalResult",
                "note": "prints the rendered command, never executes it; missing parameters are listed in missing_params. A Python entry is source, not a template: it has no parameters, its source is returned verbatim and -p is an InvalidParameter",
            },
            "pick": {
                "argv": ["hoard", "pick", "-n", "<name>", "--raw"],
                "returns": "the stored source verbatim (not JSON)",
                "note": "--json wins over --raw: combine them only if you want the pickResult payload",
            },
            "new": {
                "argv": ["hoard", "new", "-n", "<name>", "-c", "<command>", "-d", "<description>", "--json"],
                "arguments": {
                    "name": "-n/--name: unique within the namespace",
                    "command": "-c/--command: the template; {{placeholders}} are detected automatically",
                    "description": "-d/--description: one line, shown by search",
                    "namespace": "--namespace: defaults to the configured namespace",
                    "script": "--script <path>: store a Python file instead of a command (needs --name and --description)",
                },
                "returns": "#/$defs/saveResult",
                "note": "the payload is printed only after the trove was written; a failure exits non-zero and prints nothing on stdout",
            },
        },
        "$defs": {
            "compactCommand": {
                "type": "object",
                "description": "Token-dense projection: no timestamps, no tags, no empty fields.",
                "required": ["k", "c"],
                "properties": {
                    "k": {"type": "string", "description": "name"},
                    "c": {"type": "string", "description": "command template or script source"},
                    "d": {"type": "string", "description": "description, omitted when empty"},
                    "p": {"type": "array", "items": {"type": "string"}, "description": "placeholder names, omitted when none"},
                    "s": {"type": "integer", "description": "lexical match score, only on searches; higher is better"},
                },
            },
            "command": {
                "type": "object",
                "required": ["name", "command", "kind", "description", "namespace", "tags", "created", "params"],
                "properties": {
                    "name": {"type": "string"},
                    "command": {"type": "string"},
                    "kind": {"enum": ["shell", "python"]},
                    "description": {"type": "string"},
                    "namespace": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "created": {"type": ["string", "null"], "description": "YYYY-MM-DD"},
                    "params": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["name", "required"],
                            "properties": {
                                "name": {"type": "string"},
                                "required": {"type": "boolean"},
                                "default": {"type": "string"},
                            },
                        },
                    },
                },
            },
            "list": {
                "type": "object",
                "required": ["version", "commands"],
                "properties": {
                    "version": {"type": "string"},
                    "commands": {"type": "array", "items": {"$ref": "#/$defs/command"}},
                },
            },
            "compactList": {
                "type": "object",
                "required": ["commands"],
                "properties": {
                    "commands": {"type": "array", "items": {"$ref": "#/$defs/compactCommand"}},
                },
            },
            "evalResult": {
                "type": "object",
                "required": ["ok", "name", "command"],
                "properties": {
                    "ok": {"const": true},
                    "name": {"type": "string"},
                    "command": {"type": "string", "description": "the rendered command"},
                    "kind": {"enum": ["shell", "python"]},
                    "namespace": {"type": "string"},
                    "params": {
                        "type": "object",
                        "description": "parameter values that were applied",
                        "additionalProperties": {"type": "string"},
                    },
                },
            },
            "compactEvalResult": {
                "type": "object",
                "required": ["ok", "k", "c"],
                "properties": {
                    "ok": {"const": true},
                    "k": {"type": "string"},
                    "c": {"type": "string", "description": "the rendered command"},
                    "p": {
                        "type": "object",
                        "description": "applied parameter values, omitted when none",
                        "additionalProperties": {"type": "string"},
                    },
                },
            },
            "saveResult": {
                "type": "object",
                "required": ["ok", "saved", "k", "ns", "c"],
                "properties": {
                    "ok": {"const": true},
                    "saved": {"type": "string", "description": "namespace/name as stored"},
                    "k": {"type": "string"},
                    "ns": {"type": "string"},
                    "c": {"type": "string", "description": "the stored source, after secret redaction"},
                    "p": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "parameter names, omitted when none",
                    },
                    "redactions": {"type": "integer", "description": "how many secrets were masked"},
                },
            },
            "error": {
                "type": "object",
                "required": ["ok", "status", "error_type", "message"],
                "properties": {
                    "ok": {"const": false},
                    "status": {"const": "error"},
                    "error_type": {"enum": ["CommandNotFound", "InvalidParameter", "ParseError", "RuntimeError"]},
                    "message": {"type": "string"},
                    "missing_params": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "required placeholders without a value; omitted when empty",
                    },
                    "suggested_keys": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "names or parameters the caller probably meant; omitted when empty",
                    },
                },
            },
        },
    })
}

/// Exit code → error class, keyed as strings for JSON.
fn exit_codes() -> Value {
    let mut codes = Map::new();
    codes.insert(EXIT_OK.to_string(), json!("ok"));
    codes.insert(
        EXIT_PARSE.to_string(),
        json!("ParseError - the invocation itself is invalid"),
    );
    codes.insert(
        EXIT_NOT_FOUND.to_string(),
        json!("CommandNotFound - the entry or query matched nothing"),
    );
    codes.insert(
        EXIT_INVALID_PARAM.to_string(),
        json!("InvalidParameter - missing, unknown or malformed parameter"),
    );
    codes.insert(
        EXIT_RUNTIME.to_string(),
        json!("RuntimeError - I/O, database or validation failure"),
    );
    Value::Object(codes)
}

#[cfg(test)]
mod test_schema {
    use super::*;

    #[test]
    fn schema_is_valid_json_with_the_documented_definitions() {
        let schema = document();
        let defs = schema["$defs"].as_object().unwrap();
        for name in [
            "compactCommand",
            "command",
            "list",
            "compactList",
            "evalResult",
            "compactEvalResult",
            "error",
        ] {
            assert!(defs.contains_key(name), "missing $defs/{name}");
        }
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(schema["x-hoard-version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn error_shape_matches_what_the_cli_emits() {
        let schema = document();
        let fields = schema["$defs"]["error"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        for field in ["ok", "status", "error_type", "message"] {
            assert!(fields.contains(&field.to_string()), "missing {field}");
        }
        let error =
            crate::headless::CliError::new(crate::headless::ErrorType::CommandNotFound, "no entry");
        let payload = error.to_json();
        for field in &fields {
            assert!(
                payload.get(field).is_some(),
                "payload lacks required field {field}"
            );
        }
    }

    #[test]
    fn exit_codes_match_the_headless_module() {
        let schema = document();
        let codes = schema["exit_codes"].as_object().unwrap();
        assert_eq!(codes.len(), 5);
        assert!(codes.contains_key(&EXIT_OK.to_string()));
        assert!(codes.contains_key(&EXIT_PARSE.to_string()));
        assert!(codes.contains_key(&EXIT_NOT_FOUND.to_string()));
        assert!(codes.contains_key(&EXIT_INVALID_PARAM.to_string()));
        assert!(codes.contains_key(&EXIT_RUNTIME.to_string()));
    }
}
