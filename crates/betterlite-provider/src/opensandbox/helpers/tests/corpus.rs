use std::fs;

use super::super::sse::parse_lines;

#[test]
fn replays_the_opensandbox_sse_corpus() {
    let seen = fs::read_dir("fuzz/corpus/opensandbox_sse")
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .map(|body| {
            let lines: Vec<String> = body
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
                .collect();
            parse_lines(&lines)
        })
        .count();
    assert!(seen > 0_usize);
}
