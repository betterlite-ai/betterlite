#![cfg(test)]

use std::path::Path;

use crate::dispatch::runner::{Invocation, expand};
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
    };
    let argument = "{home}|{model}|{git_common_dir}|{worktree}|{schema_text}|{effort}|{schema}|{output}|{instructions}|{writable_roots}|{sandbox_settings}";
    let expanded = expand(argument, &rung, &invocation, "t").unwrap();
    assert_eq!(expanded, "/h|m|/g|/w|t|e|/s|/o|/i|[\"/g\"]|/z");
    for name in PLACEHOLDERS {
        let alone = expand(&format!("{{{name}}}"), &rung, &invocation, "t").unwrap();
        assert!(!alone.contains('{'), "{name} was not expanded");
    }
}
