use std::io;
use std::io::Write;
use std::sync::Arc;

use num_traits::ToPrimitive as _;
use tokio::runtime::Builder;

use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credentials::notice::keychain_unreadable_notice;
use crate::keyring::SecretVault;
use crate::pkce_login::WallClock;
use crate::token_records::record::CliTokenRecord;

use super::error::SessionError;
use super::login_command;

pub const WHOAMI_ABOUT: &str = "Show current authentication status";

const SECONDS_PER_HOUR: f64 = 3_600.0;
const NOT_AUTHENTICATED: &[u8] = b"Not authenticated. Run 'lite login' to authenticate.\n";

pub struct WhoamiRequest<'request, W> {
    pub files: &'request dyn CredentialFiles,
    pub vault: &'request mut (dyn SecretVault + Send + Sync),
    pub clock: Arc<W>,
    pub expiration_hours: i64,
    pub stdout: &'request mut dyn Write,
}

pub fn whoami<W>(request: &mut WhoamiRequest<'_, W>) -> Result<(), SessionError>
where
    W: WallClock + Send + Sync + 'static,
{
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| SessionError::Runtime { source })?;
    let loaded = runtime.block_on(load_cli_token(request.files, &mut *request.vault));
    let Some(record) = loaded else {
        let _write = request.stdout.write_all(NOT_AUTHENTICATED);
        return Ok(());
    };
    let _rendered = render(
        &record,
        &*request.vault,
        &*request.clock,
        request.expiration_hours,
        &mut *request.stdout,
    );
    Ok(())
}

fn render(
    record: &CliTokenRecord,
    vault: &dyn SecretVault,
    clock: &dyn WallClock,
    expiration_hours: i64,
    stdout: &mut dyn Write,
) -> io::Result<()> {
    render_identity(record, stdout)?;
    let now = clock.now_seconds().to_f64().unwrap_or_default();
    let age = hours_between(now, record.timestamp);
    writeln!(stdout, "Token age: {age:.1} hours")?;
    if record.key.is_none() {
        let notice = keychain_unreadable_notice(vault);
        writeln!(stdout, "{notice}")?;
    }
    render_expiry(record, now, age, expiration_hours, stdout)
}

fn render_identity(record: &CliTokenRecord, stdout: &mut dyn Write) -> io::Result<()> {
    let status = if record.key.is_some() {
        "Authenticated"
    } else {
        "Signed in, but the credential cannot be read"
    };
    writeln!(stdout, "{status}")?;
    writeln!(stdout, "User Email: {}", or_unknown(&record.user_email))?;
    writeln!(stdout, "User ID: {}", or_unknown(&record.user_id))?;
    writeln!(stdout, "User Role: {}", or_unknown(&record.user_role))?;
    record
        .team_id
        .as_deref()
        .filter(|team| !team.is_empty())
        .map_or(Ok(()), |team| writeln!(stdout, "Team ID: {team}"))
}

const fn or_unknown(value: &str) -> &str {
    if value.is_empty() { "Unknown" } else { value }
}

fn render_expiry(
    record: &CliTokenRecord,
    now: f64,
    age: f64,
    expiration_hours: i64,
    stdout: &mut dyn Write,
) -> io::Result<()> {
    let Some(expires_at) = record.expires_at else {
        return warn_when_aged(age, expiration_hours, stdout);
    };
    let remaining = hours_between(expires_at, now);
    if remaining <= 0.0_f64 {
        let command = login_command(record.refresh_token.is_some());
        return writeln!(stdout, "Key expired. Run '{command}' again");
    }
    let suffix = if record.refresh_token.is_some() {
        ", renewed on next use"
    } else {
        ""
    };
    writeln!(stdout, "Key expires in: {remaining:.1} hours{suffix}")
}

fn warn_when_aged(age: f64, expiration_hours: i64, stdout: &mut dyn Write) -> io::Result<()> {
    let limit = expiration_hours.to_f64().unwrap_or_default();
    if age > limit {
        return writeln!(
            stdout,
            "Warning: Token is more than {expiration_hours} hours old and may have expired."
        );
    }
    Ok(())
}

const fn hours_between(later: f64, earlier: f64) -> f64 {
    // float_arithmetic is denied; a fused multiply-add gives (later - earlier) / 3600
    earlier
        .mul_add(-1.0_f64, later)
        .mul_add(SECONDS_PER_HOUR.recip(), 0.0_f64)
}
