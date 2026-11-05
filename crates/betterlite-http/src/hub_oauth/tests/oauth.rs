use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::time::sleep;

use super::support::{Env, Logs, body_of, header, implicit, oidc_settings};
use crate::hub_oauth::error::StoredTokensError;
use crate::hub_token::{HubTokenError, HubTokenResolver};

const NOW: i64 = 1_700_000_000;
const EXPIRES_SOON: i64 = 1_700_003_600;
const EXPIRES_IN_TWO_DAYS: i64 = 1_700_172_800;
const EXPIRES_IN_TEN_DAYS: i64 = 1_700_864_000;
const CLIENT_ID: &str = "26be6b09-91c5-47da-9861-d2d2bb7a7e36";
const OTHER_SECTION: &str = "[other]\nhf_token = zzz\n\n";
const REFRESHED_ONE_DAY: &str =
    r#"{"access_token":"new","refresh_token":"rt","expires_in":100000}"#;

struct Fixture {
    env: Env,
    lookup: HubTokenResolver,
    token: PathBuf,
    stored: PathBuf,
    lock_file: PathBuf,
}

impl Fixture {
    async fn new(stored_tokens: &str) -> Self {
        let env = Env::new().await;
        let hub = env.hub();
        fs::create_dir_all(hub.token_path.parent().unwrap()).unwrap();
        fs::write(&hub.token_path, "old\n").unwrap();
        fs::write(&hub.stored_tokens_path, stored_tokens).unwrap();
        let token = hub.token_path.clone();
        let stored = hub.stored_tokens_path.clone();
        let lock_file = stored.with_file_name("stored_tokens.lock");
        let lookup = env.lookup(hub, oidc_settings(None));
        Self {
            env,
            lookup,
            token,
            stored,
            lock_file,
        }
    }

    fn lock(&self) -> File {
        let lock = File::create(&self.lock_file).unwrap();
        lock.lock().unwrap();
        lock
    }
}

fn stored(token: &str, refresh_token: &str, expires_at: i64) -> String {
    format!(
        "[work]\nhf_token = {token}\nrefresh_token = {refresh_token}\nexpires_at = {expires_at}\n"
    )
}

pub(super) async fn token_is_refreshed_a_day_before_it_ends() {
    let fixture = Fixture::new(&stored("old", "rt", EXPIRES_IN_TWO_DAYS)).await;
    fixture.env.server.answer(200, REFRESHED_ONE_DAY);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    fixture.env.clock.advance(86_399);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert!(fixture.env.server.requests().is_empty());
    fixture.env.clock.advance(1);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "new");
    assert_eq!(fixture.env.server.requests().len(), 1);
}

pub(super) async fn tokens_without_usable_metadata_are_returned_as_they_are() {
    for contents in [
        "[work]\nhf_token = old\n",
        "[work]\nhf_token = old\nrefresh_token = rt\n",
        "[work]\nhf_token = old\nexpires_at = soon\nrefresh_token = rt\n",
        "[work]\nhf_token = other\nrefresh_token = rt\nexpires_at = 1\n",
    ] {
        let fixture = Fixture::new(contents).await;
        assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
        assert!(fixture.env.server.requests().is_empty());
    }
}

pub(super) async fn refresh_request_is_a_form_post() {
    let fixture = Fixture::new(&stored("old", "rt", EXPIRES_SOON)).await;
    fixture.env.server.answer(200, REFRESHED_ONE_DAY);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "new");
    let requests = fixture.env.server.requests();
    let request = requests.first().unwrap();
    assert!(request.starts_with("POST /oauth/token HTTP/1.1\r\n"));
    assert_eq!(
        header(request, "content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        body_of(request),
        format!("grant_type=refresh_token&refresh_token=rt&client_id={CLIENT_ID}")
    );
}

pub(super) async fn refreshed_token_is_saved_with_private_modes() {
    let fixture = Fixture::new(&format!(
        "{OTHER_SECTION}{}",
        stored("old", "rt", EXPIRES_SOON)
    ))
    .await;
    fixture.env.server.answer(
        200,
        r#"{"access_token":"new","refresh_token":"rt2","expires_in":7200}"#,
    );
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "new");

    assert_eq!(fs::read_to_string(&fixture.token).unwrap(), "new");
    assert_eq!(
        fs::read_to_string(&fixture.stored).unwrap(),
        format!(
            "{OTHER_SECTION}[work]\nhf_token = new\nrefresh_token = rt2\nexpires_at = 1700007200\n\n"
        )
    );
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fixture.token), 0o600);
    assert_eq!(mode(&fixture.stored), 0o600);
    assert_eq!(mode(fixture.token.parent().unwrap()), 0o700);
    assert_eq!(mode(&fixture.lock_file), 0o664);
}

pub(super) async fn refreshed_token_is_rechecked_after_five_minutes() {
    let fixture = Fixture::new(&stored("old", "rt", EXPIRES_SOON)).await;
    fixture.env.server.answer(
        200,
        r#"{"access_token":"new","refresh_token":"rt2","expires_in":7200}"#,
    );
    fixture
        .env
        .server
        .answer(200, r#"{"access_token":"newer"}"#);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "new");
    fixture.env.clock.advance(299);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "new");
    assert_eq!(fixture.env.server.requests().len(), 1);
    fixture.env.clock.advance(1);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "newer");

    let requests = fixture.env.server.requests();
    assert!(body_of(&requests[1]).contains("refresh_token=rt2"));
    // no refresh token and no lifetime in the answer keeps the old refresh token and stores no expiry
    assert_eq!(
        fs::read_to_string(&fixture.stored).unwrap(),
        "[work]\nhf_token = newer\nrefresh_token = rt2\n\n"
    );
}

pub(super) async fn failed_refresh_warns_once() {
    let (logs, _guard) = Logs::capture();
    let fixture = Fixture::new(&stored("old", "rt", NOW)).await;
    fixture.env.server.answer(500, "oops");
    fixture
        .env
        .server
        .answer(503, r#"{"error":"temporarily_unavailable"}"#);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert_eq!(fixture.env.server.requests().len(), 1);
    assert_eq!(logs.text().matches("Could not refresh").count(), 1);
    assert_eq!(fs::read_to_string(&fixture.token).unwrap(), "old\n");

    fixture.env.clock.advance(300);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert_eq!(fixture.env.server.requests().len(), 2);
    assert_eq!(logs.text().matches("Could not refresh").count(), 1);
}

pub(super) async fn invalid_grant_warns_and_is_never_rechecked() {
    let (logs, _guard) = Logs::capture();
    let fixture = Fixture::new(&stored("old", "rt", NOW)).await;
    fixture.env.server.answer(500, "oops");
    fixture.env.server.answer(
        400,
        r#"{"error":"invalid_grant","error_description":"gone"}"#,
    );
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    fixture.env.clock.advance(300);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    let text = logs.text();
    assert!(text.contains("hf auth login"), "{text}");
    assert!(text.contains("invalid_grant - gone"), "{text}");

    fixture.env.clock.advance(315_360_000);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert_eq!(fixture.env.server.requests().len(), 2);
}

pub(super) async fn lock_timeout_keeps_the_stored_token() {
    let (logs, _guard) = Logs::capture();
    let mut fixture = Fixture::new(&stored("old", "rt", NOW)).await;
    fixture.lookup.oauth.lock_timeout = Duration::from_millis(200);
    let held = fixture.lock();
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert!(fixture.env.server.requests().is_empty());
    assert!(logs.text().contains("could not be acquired"));

    drop(held);
    fixture.env.clock.advance(300);
    fixture.env.server.answer(200, REFRESHED_ONE_DAY);
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "new");
}

pub(super) async fn refresh_by_another_process_is_adopted() {
    let fixture = Fixture::new(&stored("old", "rt", NOW)).await;
    let held = fixture.lock();
    let other_process = async {
        sleep(Duration::from_millis(300)).await;
        fs::write(&fixture.stored, stored("other", "rt9", EXPIRES_IN_TEN_DAYS)).unwrap();
        drop(held);
    };
    let (token, ()) = tokio::join!(implicit(&fixture.lookup), other_process);
    assert_eq!(token.unwrap().unwrap(), "other");
    assert!(fixture.env.server.requests().is_empty());
    assert_eq!(fs::read_to_string(&fixture.token).unwrap(), "old\n");
}

pub(super) async fn token_file_that_moved_meanwhile_is_not_overwritten() {
    let fixture = Fixture::new(&stored("old", "rt", NOW)).await;
    fixture.env.server.answer(200, REFRESHED_ONE_DAY);
    let held = fixture.lock();
    let switcher = async {
        sleep(Duration::from_millis(300)).await;
        fs::write(&fixture.token, "switched").unwrap();
        drop(held);
    };
    let (token, ()) = tokio::join!(implicit(&fixture.lookup), switcher);
    assert_eq!(token.unwrap().unwrap(), "new");
    assert_eq!(fs::read_to_string(&fixture.token).unwrap(), "switched");
    assert!(
        fs::read_to_string(&fixture.stored)
            .unwrap()
            .contains("hf_token = new")
    );
}

pub(super) async fn stored_tokens_that_are_not_utf8_are_an_error() {
    let fixture = Fixture::new("").await;
    fs::write(&fixture.stored, b"[work]\nhf_token = \xff\n").unwrap();
    assert!(matches!(
        implicit(&fixture.lookup).await,
        Err(HubTokenError::StoredTokens(StoredTokensError::Utf8 { .. }))
    ));
}

pub(super) async fn unparsable_stored_tokens_count_as_empty() {
    let (logs, _guard) = Logs::capture();
    let fixture = Fixture::new("not an ini file\n").await;
    assert_eq!(implicit(&fixture.lookup).await.unwrap().unwrap(), "old");
    assert!(logs.text().contains("Error parsing stored tokens file"));
}
