const WALL_CLOCK_FLOOR: i64 = 1_700_000_000;

#[test]
fn azure_runtime_the_clocks_read_the_wall_clock() {
    let clock = ProductionClock;
    assert!(TokenClock::now_unix_seconds(&clock) > WALL_CLOCK_FLOOR);
    assert!(OidcClock::now_unix_seconds(&clock) > 1_700_000_000.0_f64);
    let host = ProductionHost {
        environment: BTreeMap::new(),
    };
    assert!(ArcHost::now_unix_seconds(&host) > WALL_CLOCK_FLOOR);
}

#[cfg(target_os = "linux")]
#[test]
fn azure_runtime_the_host_reports_its_platform() {
    let host = ProductionHost {
        environment: BTreeMap::new(),
    };
    assert_eq!(host.platform(), ArcPlatform::Linux);
}

#[test]
fn azure_runtime_the_host_reads_its_environment() {
    let host = ProductionHost {
        environment: BTreeMap::from([
            ("AZURE_CLIENT_ID".to_owned(), "client".to_owned()),
            ("ProgramData".to_owned(), "/var/lib".to_owned()),
            ("ProgramFiles".to_owned(), "/opt".to_owned()),
        ]),
    };
    assert_eq!(
        host.environment("AZURE_CLIENT_ID").as_deref(),
        Some("client")
    );
    assert_eq!(host.environment("ABSENT"), None);
    assert_eq!(host.program_data(), "/var/lib");
    assert_eq!(host.program_files(), "/opt");
}

#[test]
fn azure_runtime_the_host_reads_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("arc.json");
    fs::write(&path, b"abcdef").unwrap();
    let host = ProductionHost {
        environment: BTreeMap::new(),
    };
    let present = path.to_str().unwrap().to_owned();
    assert_eq!(host.file_size(&present).unwrap(), 6);
    assert_eq!(host.read_text(&present).unwrap(), "abcdef");
    let missing = directory.path().join("absent").to_str().unwrap().to_owned();
    assert_eq!(
        host.file_size(&missing).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        host.read_text(&missing).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_runtime_the_scheduler_waits_for_its_delay() {
    let runtime = Handle::current();
    let elapsed = run_blocking(move || {
        let scheduler = ThreadScheduler { runtime };
        let started = Instant::now();
        Scheduler::sleep(&scheduler, Duration::from_millis(30));
        started.elapsed()
    })
    .await;
    assert!(elapsed >= Duration::from_millis(30));
}
