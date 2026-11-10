#![cfg(test)]

use std::time::Duration;

use tempfile::TempDir;

use crate::codex_settings::version::installed_codex_version;

#[cfg(unix)]
use crate::codex_settings::tests::support::write_script;

#[cfg(unix)]
#[tokio::test]
async fn codex_probe_reports_the_installed_version() {
    let home = TempDir::new().unwrap();
    let good = write_script(&home, "codex", "#!/bin/sh\nprintf 'codex-cli 0.129.0\\n'\n");
    let reported = installed_codex_version(&good, Duration::from_secs(5)).await;
    assert_eq!(reported.as_deref(), Some("codex-cli 0.129.0\n"));
    let failing = write_script(&home, "failing", "#!/bin/sh\nexit 3\n");
    assert!(
        installed_codex_version(&failing, Duration::from_secs(5))
            .await
            .is_none()
    );
    let slow = write_script(&home, "slow", "#!/bin/sh\nsleep 2\n");
    assert!(
        installed_codex_version(&slow, Duration::from_millis(100))
            .await
            .is_none()
    );
}
