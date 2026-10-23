#![cfg(test)]

use crate::builtins::globs::glob_set;
use crate::coverage::hits;

#[test]
fn every_path_and_every_glob_is_accounted_for() {
    let patterns = [
        "litellm/llms/anthropic/**".to_owned(),
        "litellm/llms/missing/**".to_owned(),
    ];
    let paths = [
        "litellm/llms/anthropic/chat/handler.py".to_owned(),
        "litellm/llms/openai/openai.py".to_owned(),
    ];
    let (matched, dead) = hits(&glob_set(&patterns).unwrap(), &patterns, &paths);
    assert_eq!(matched, [true, false]);
    assert_eq!(dead, ["litellm/llms/missing/**"]);
}
