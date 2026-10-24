#![cfg(test)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, process};

use rstest::rstest;
use tokio::{fs as async_fs, time};

use super::child::ProbeCommand;
use super::operation::{EchoInput, EchoOutput, ProbeOperation, ProbeOutput};
use super::{ProbeClock, ProbeReply, ProbeRequest, ProbeSample, parse_message};
use crate::OracleError;

mod b392_behaviors;
mod b392_boundaries;

const REQUEST: &str = r#"{"clock":{"unix_millis":1700000000123,"monotonic_millis":42000},"samples":[0,9007199254740991],"operation":{"echo":{"text":"ping","draws":2,"warnings":["stale health snapshot"]}}}"#;
const REPLY: &str = r#"{"outcome":"ok","output":{"echo":{"text":"ping","clock":{"unix_millis":1700000000123,"monotonic_millis":42000},"samples":[0,9007199254740991]}},"warnings":["stale health snapshot"]}"#;

fn request() -> ProbeRequest {
    let largest = ProbeSample::try_from((1_u64 << 53_u32) - 1).unwrap();
    ProbeRequest {
        clock: ProbeClock {
            unix_millis: 1_700_000_000_123,
            monotonic_millis: 42_000,
        },
        samples: vec![ProbeSample::try_from(0).unwrap(), largest],
        operation: ProbeOperation::Echo(EchoInput {
            text: "ping".to_owned(),
            draws: 2,
            warnings: vec!["stale health snapshot".to_owned()],
        }),
    }
}

fn echo_reply() -> ProbeReply {
    let echo = request();
    ProbeReply::Ok {
        output: ProbeOutput::Echo(EchoOutput {
            text: "ping".to_owned(),
            clock: echo.clock,
            samples: echo.samples,
        }),
        warnings: vec!["stale health snapshot".to_owned()],
    }
}

fn scratch(name: &str) -> PathBuf {
    env::temp_dir().join(format!("betterlite-b392-{name}-{}", process::id()))
}

fn shell(script: &str, args: &[&str]) -> ProbeCommand {
    let mut arguments = vec![
        OsString::from("-c"),
        OsString::from(script),
        OsString::from("probe"),
    ];
    arguments.extend(args.iter().map(OsString::from));
    ProbeCommand {
        program: PathBuf::from("sh"),
        args: arguments,
        cwd: env::temp_dir(),
    }
}

async fn read_pid(path: &Path) -> String {
    loop {
        if let Ok(pid) = async_fs::read_to_string(path).await
            && pid.ends_with('\n')
        {
            return pid.trim().to_owned();
        }
        time::sleep(Duration::from_millis(10)).await;
    }
}

async fn assert_gone(pid: &str) {
    for _ in 0..200_u32 {
        match async_fs::read_to_string(format!("/proc/{pid}/stat")).await {
            Err(_) => return,
            Ok(stat) if stat.contains(") Z ") => return,
            Ok(_) => time::sleep(Duration::from_millis(10)).await,
        }
    }
    panic!("probe process {pid} outlived its owner");
}

#[rstest]
fn replays_the_probe_ipc_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/probe_ipc");
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let parsed = parse_message::<ProbeRequest>(&bytes);
        let _reply = parse_message::<ProbeReply>(&bytes);
        let name = path.file_name().unwrap().to_str().unwrap();
        if name.ends_with("-valid.json") {
            assert_eq!(parsed.as_ref().unwrap(), &request());
        }
        if name.ends_with("-invalid.json") {
            assert!(matches!(parsed, Err(OracleError::ProbeMalformed(_))));
        }
    }
}
