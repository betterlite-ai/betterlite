#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::parse_helper_case;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

#[test]
fn helper_seeds_select_bedrock_http_stream() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds = fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_http_stream"))
        })
        .collect::<Vec<_>>();
    assert!(!seeds.is_empty());
    for path in &seeds {
        let source = fs::read_to_string(path).unwrap();
        let case = parse_helper_case(path, &source).unwrap();
        assert_eq!(case.call.operation, "bedrock_http_stream");
    }
}
