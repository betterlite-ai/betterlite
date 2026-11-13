use betterlite_cli::settings_backup::read::read_backup as read_backup_file;
use betterlite_cli::settings_backup::report::restore_and_report;
use betterlite_cli::token_records::record::CliTokenRecord;
use betterlite_cli::up::command::run_up;
use betterlite_cli::up::login::ensure_fresh_login;
use serde::Deserialize;
use serde_json::{Value, json};

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

fn failure<M>(error: &M, root: &str, class: &str) -> Value
where
    M: std::fmt::Display,
{
    json!({"class": class, "message": error.to_string().replace(root, "<tmp>")})
}

fn fresh_login(input: &Input) -> HelperResult {
    let mut world = World::new(input)?;
    let mut login = world.scripted_login(input);
    let stop = world.scripted_stop();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = {
        let mut ports = world.bind(&mut login, &stop, &mut stderr, &mut stdout);
        ensure_fresh_login(&mut ports)
    };
    let error = outcome
        .err()
        .map(|error| failure(&error, &world.root_text(), "UpError"));
    Ok(world.answer(&stdout, error))
}

fn read_backup(input: &Input) -> HelperResult {
    let world = World::new(input)?;
    let outcome = read_backup_file(&world.backup_path());
    let error = outcome
        .err()
        .map(|error| failure(&error, &world.root_text(), "UpError"));
    Ok(world.answer(&[], error))
}

fn down(input: &Input) -> HelperResult {
    let world = World::new(input)?;
    let mut stdout = Vec::new();
    let outcome = restore_and_report(world.home_path(), &world.private_files, &mut stdout);
    let error = outcome
        .err()
        .map(|error| failure(&error, &world.root_text(), "ClickException"));
    Ok(world.answer(&stdout, error))
}

fn up(input: &Input) -> HelperResult {
    let mut world = World::new(input)?;
    let mut login = world.scripted_login(input);
    let stop = world.scripted_stop();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = {
        let mut ports = world.bind(&mut login, &stop, &mut stderr, &mut stdout);
        run_up(&mut ports)
    };
    let error = outcome
        .err()
        .map(|error| failure(&error, &world.root_text(), "ClickException"));
    Ok(world.answer(&stdout, error))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.operation {
            Operation::FreshLogin => fresh_login(input),
            Operation::ReadBackup => read_backup(input),
            Operation::Down => down(input),
            Operation::Up => up(input),
        }
    }
}
