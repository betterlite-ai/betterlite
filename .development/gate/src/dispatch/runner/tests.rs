#![cfg(test)]

use std::path::Path;

use crate::dispatch::runner::{Invocation, Reply, claude_reply, expand};
use crate::spec::agents::{PLACEHOLDERS, Rung};

#[test]
fn every_placeholder_gets_its_own_value() {
    let rung = Rung {
        runner: "codex-worker".to_owned(),
        model: "m".to_owned(),
        effort: "e".to_owned(),
        attempts: 1,
        timeout_minutes: 1,
    };
    let invocation = Invocation {
        worktree: Path::new("/w"),
        git_common_dir: Path::new("/g"),
        home: Path::new("/h"),
        sandbox_settings: Path::new("/z"),
        writable_roots: "[\"/g\"]",
        schema: Path::new("/s"),
        output: Path::new("/o"),
        instructions: Path::new("/i"),
        log: Path::new("/l"),
        prompt: "",
        session: Some("s"),
    };
    let argument = "{home}|{model}|{git_common_dir}|{worktree}|{schema_text}|{effort}|{schema}|{output}|{instructions}|{writable_roots}|{sandbox_settings}|{session}";
    let expanded = expand(argument, &rung, &invocation, "t").unwrap();
    assert_eq!(expanded, "/h|m|/g|/w|t|e|/s|/o|/i|[\"/g\"]|/z|s");
    for name in PLACEHOLDERS {
        let alone = expand(&format!("{{{name}}}"), &rung, &invocation, "t").unwrap();
        assert!(!alone.contains('{'), "{name} was not expanded");
    }
}

#[test]
fn a_structured_answer_is_the_claude_answer() {
    let text = r#"{"structured_output":{"task":"B171"},"result":"done"}"#;
    assert_eq!(
        claude_reply(text).unwrap().answer.as_deref(),
        Some(r#"{"task":"B171"}"#)
    );
}

#[test]
fn a_json_result_stands_in_when_the_structured_output_is_missing() {
    let text = r#"{"result":"{\"task\":\"B171\",\"status\":\"done\"}","stop_reason":"tool_use"}"#;
    assert_eq!(
        claude_reply(text).unwrap().answer.as_deref(),
        Some(r#"{"task":"B171","status":"done"}"#)
    );
}

#[test]
fn a_prose_result_is_no_answer() {
    let text = r#"{"result":"I could not finish."}"#;
    assert_eq!(claude_reply(text).unwrap().answer, None);
}

#[test]
fn empty_stdout_after_a_timeout_is_no_answer() {
    let nothing = Reply {
        answer: None,
        session: None,
    };
    assert_eq!(claude_reply("").unwrap(), nothing);
    assert_eq!(claude_reply("\n").unwrap(), nothing);
}

#[test]
fn the_session_id_comes_back_with_the_answer() {
    let text = r#"{"structured_output":{"task":"B171"},"session_id":"cdaf2939"}"#;
    assert_eq!(
        claude_reply(text).unwrap(),
        Reply {
            answer: Some(r#"{"task":"B171"}"#.to_owned()),
            session: Some("cdaf2939".to_owned()),
        }
    );
}
