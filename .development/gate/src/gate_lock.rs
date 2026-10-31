mod tests;

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::GateError;

pub(crate) const GRACE: Duration = Duration::from_secs(120);

fn own_group() -> Result<String, GateError> {
    let path = Path::new("/proc/self/stat");
    let stat = fs::read_to_string(path).map_err(|source| GateError::Read {
        path: path.to_owned(),
        source,
    })?;
    // the command name may hold spaces and brackets, so the fields after it start at the last ')'
    let fields = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
    fields
        .split_whitespace()
        .nth(2)
        .map(str::to_owned)
        .ok_or_else(|| GateError::Read {
            path: path.to_owned(),
            source: io::Error::other("no process group field"),
        })
}

pub(crate) fn hold(log_dir: &Path) -> Result<File, GateError> {
    let path = log_dir.join("gate.lock");
    // no truncation before the lock: a waiting run must not wipe the group the holder wrote
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| GateError::Write {
            path: path.clone(),
            source,
        })?;
    file.lock().map_err(|source| GateError::Lock {
        path: path.clone(),
        source,
    })?;
    let group = own_group()?;
    file.set_len(0)
        .and_then(|()| file.write_all(group.as_bytes()))
        .map_err(|source| GateError::Write { path, source })?;
    Ok(file)
}

fn signal(group: &str, name: &str) -> Result<(), GateError> {
    // kill fails for a group that is already gone, so the caller waits on the lock instead
    Command::new("kill")
        .args(["-s", name, "--", &format!("-{group}")])
        .stderr(Stdio::null())
        .status()
        .map_err(|source| GateError::Spawn {
            program: "kill".to_owned(),
            source,
        })?;
    Ok(())
}

fn is_free(file: &File, path: &Path) -> Result<bool, GateError> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(source)) => Err(GateError::Lock {
            path: path.to_owned(),
            source,
        }),
    }
}

pub(crate) fn stop_orphan(log_dir: &Path, grace: Duration) -> Result<(), GateError> {
    let path = log_dir.join("gate.lock");
    if !path.exists() {
        return Ok(());
    }
    let read_error = |source| GateError::Read {
        path: path.clone(),
        source,
    };
    let file = File::open(&path).map_err(read_error)?;
    if is_free(&file, &path)? {
        return Ok(());
    }
    let text = fs::read_to_string(&path).map_err(read_error)?;
    let holder = text.trim();
    // a holder in this process's own group is not an orphan, so the gate waits for it instead
    let orphan = holder.parse::<u32>().is_ok() && holder != own_group()?;
    if orphan {
        signal(holder, "INT")?;
    }
    let started = Instant::now();
    while orphan && !is_free(&file, &path)? {
        if started.elapsed() >= grace {
            signal(holder, "KILL")?;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    // the lock goes free once every process that holds it has exited
    file.lock()
        .map_err(|source| GateError::Lock { path, source })
}
