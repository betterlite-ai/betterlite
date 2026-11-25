use betterlite_db::database_faults::reconnect::DbReconnect as _;

use crate::db_settings_reload::ports::NoReconnect;

#[tokio::test]
async fn a_no_reconnect_handle_has_no_timeouts_and_never_reconnects() {
    let mut reconnect = NoReconnect;
    assert_eq!(reconnect.reconnect_timeout_seconds(), None);
    assert_eq!(reconnect.reconnect_lock_timeout_seconds(), None);
    assert!(
        !reconnect
            .attempt_db_reconnect("init_sso_settings_in_db_lookup_failure", 2.0_f64, 0.1_f64)
            .await
            .unwrap()
    );
}
