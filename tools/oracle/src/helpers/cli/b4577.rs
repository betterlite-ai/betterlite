use betterlite_cli::settings_backup::read::read_backup as read_backup_file;
use betterlite_cli::settings_backup::report::restore_and_report;
use betterlite_cli::token_records::record::CliTokenRecord;
use betterlite_cli::up::command::run_up;
use betterlite_cli::up::login::ensure_fresh_login;
use serde::Deserialize;
use serde_json::json;

use self::fixtures::World;
use crate::helpers::{Helper, HelperResult};

mod agents;
mod fakes;
mod fixtures;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    stored: Option<CliTokenRecord>,
    #[serde(default)]
    login_token: Option<CliTokenRecord>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_key_from_token_file: bool,
    #[serde(default)]
    tty: bool,
    #[serde(default)]
    settings: Option<String>,
    #[serde(default)]
    backup: Option<String>,
    #[serde(default)]
    verify_status: Option<u16>,
    #[serde(default)]
    stop: Option<StopKind>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    FreshLogin,
    ReadBackup,
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopKind {
    Sigint,
    Sigterm,
    Atexit,
}

fn finish<M>(world: &World, stdout: &[u8], outcome: Result<(), M>, click: bool) -> serde_json::Value
where
    M: std::fmt::Display,
{
    let error = outcome.err().map(|failure| {
        let class = if click { "ClickException" } else { "UpError" };
        let root = world.home.path().display().to_string();
        json!({"class": class, "message": failure.to_string().replace(&root, "<tmp>")})
    });
    world.answer(stdout, error.as_ref())
}

fn drive(input: &Input) -> HelperResult {
    let mut world = World::new(input)?;
    let mut login = world.scripted_login(input);
    let stop = world.scripted_stop();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let fresh = matches!(input.operation, Operation::FreshLogin);
    let outcome = {
        let mut ports = world.bind(&mut login, &stop, &mut stderr, &mut stdout);
        if fresh {
            ensure_fresh_login(&mut ports)
        } else {
            run_up(&mut ports)
        }
    };
    Ok(finish(&world, &stdout, outcome, !fresh))
}

fn report(input: &Input) -> HelperResult {
    let world = World::new(input)?;
    let mut stdout = Vec::new();
    let down = matches!(input.operation, Operation::Down);
    let outcome = if down {
        restore_and_report(world.home.path(), &world.private_files, &mut stdout)
    } else {
        let backup = world
            .home
            .path()
            .join(".litellm")
            .join("claude_settings_backup.json");
        read_backup_file(&backup).map(|_record| ())
    };
    Ok(finish(&world, &stdout, outcome, down))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        if matches!(input.operation, Operation::FreshLogin | Operation::Up) {
            return drive(input);
        }
        report(input)
    }
}
