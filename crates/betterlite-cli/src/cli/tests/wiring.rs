use crate::cli::Failure;
use crate::cli::ports::wiring::token_path;
use crate::cli::tests::support::{Captured, FakeEnvironment, FakeVault, proxy_context, session};

#[test]
fn a_session_with_a_home_resolves_the_token_path_from_it() {
    let home = tempfile::tempdir().unwrap();
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let mut vault = FakeVault;
    let out = Captured::default();
    let err = Captured::default();
    let context = proxy_context("http://proxy.example");
    let session = session(&context, &environment, &mut vault, &out, &err);
    let path = token_path(&session).unwrap();
    assert!(
        path.starts_with(home.path()),
        "{path:?} is outside the home"
    );
}

#[test]
fn a_session_without_a_home_has_no_token_path() {
    let environment = FakeEnvironment::default();
    let mut vault = FakeVault;
    let out = Captured::default();
    let err = Captured::default();
    let context = proxy_context("http://proxy.example");
    let session = session(&context, &environment, &mut vault, &out, &err);
    let failure = token_path(&session).unwrap_err();
    assert!(matches!(failure, Failure::Plain(_)));
}
