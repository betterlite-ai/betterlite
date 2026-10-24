#![cfg(test)]

use crate::coverage::tally;

#[test]
fn every_path_and_every_glob_is_accounted_for() {
    let patterns = [
        "litellm/proxy/**".to_owned(),
        "litellm/llms/anthropic/**".to_owned(),
        "litellm/llms/anthropic/chat/*.py".to_owned(),
        "litellm/llms/missing/**".to_owned(),
    ];
    let owners = [None, Some(0), Some(0), Some(1)];
    let paths = vec![
        "litellm/llms/anthropic/chat/handler.py".to_owned(),
        "litellm/llms/openai/openai.py".to_owned(),
        "litellm/proxy/proxy_server.py".to_owned(),
    ];
    let result = tally(&patterns, &owners, paths, 2).unwrap();
    assert_eq!(result.counts, [1, 0]);
    assert_eq!(result.uncovered, ["litellm/llms/openai/openai.py"]);
    assert_eq!(result.used, [true, true, true, false]);
}
