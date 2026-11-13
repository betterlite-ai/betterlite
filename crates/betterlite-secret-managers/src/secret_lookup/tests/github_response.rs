use std::path::Path;

use crate::secret_lookup::github_response::decode_github_token;

#[test]
fn replays_the_github_oidc_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/github_oidc_response");
    let results = fs::read_dir(corpus)
        .expect("the corpus folder")
        .map(|entry| fs::read(entry.expect("a corpus entry").path()).expect("a corpus file"))
        .map(|bytes| decode_github_token(&bytes))
        .collect::<Vec<_>>();
    assert!(results.len() >= 8);
    assert!(results.iter().any(|result| matches!(result, Ok(Some(_)))));
    assert!(results.iter().any(|result| matches!(result, Ok(None))));
    assert!(results.iter().any(Result::is_err));
}
