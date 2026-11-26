use betterlite_config::loader::EnvironmentLookup;

use super::error::SharedHealthSettingsError;

pub const DEFAULT_HEALTH_CHECK_TTL: i64 = 300;
pub const DEFAULT_LOCK_TTL: i64 = 60;
pub const HEALTH_CHECK_TTL_VARIABLE: &str = "DEFAULT_SHARED_HEALTH_CHECK_TTL";
pub const LOCK_TTL_VARIABLE: &str = "DEFAULT_SHARED_HEALTH_CHECK_LOCK_TTL";

// SharedHealthCheckManager.__init__'s two TTLs, resolved once at startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedHealthSettings {
    pub health_check_ttl: i64,
    pub lock_ttl: i64,
}

pub const DEFAULT_SETTINGS: SharedHealthSettings = SharedHealthSettings {
    health_check_ttl: DEFAULT_HEALTH_CHECK_TTL,
    lock_ttl: DEFAULT_LOCK_TTL,
};

impl SharedHealthSettings {
    // The source reads both defaults from os.environ at import, so an unparsable value stops the start.
    pub fn resolve<L>(environment: &L) -> Result<Self, SharedHealthSettingsError>
    where
        L: EnvironmentLookup + ?Sized,
    {
        Ok(Self {
            health_check_ttl: ttl(
                environment,
                HEALTH_CHECK_TTL_VARIABLE,
                DEFAULT_HEALTH_CHECK_TTL,
            )?,
            lock_ttl: ttl(environment, LOCK_TTL_VARIABLE, DEFAULT_LOCK_TTL)?,
        })
    }
}

fn ttl<L>(
    environment: &L,
    variable: &'static str,
    default: i64,
) -> Result<i64, SharedHealthSettingsError>
where
    L: EnvironmentLookup + ?Sized,
{
    let Some(text) = environment.lookup(variable) else {
        return Ok(default);
    };
    Ok(betterlite_env::number::<i64>(variable, &text)?.unwrap_or(default))
}
