use betterlite_config::sections::worker_options_reload::LoopType;

// the sys.platform names LiteLLM treats as unable to run uvloop
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SysPlatform {
    Win32,
    Cygwin,
    Cli,
    Other,
}

impl SysPlatform {
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Win32
        } else {
            Self::Other
        }
    }
}

pub const fn resolve_loop_type(platform: SysPlatform) -> Option<LoopType> {
    match platform {
        SysPlatform::Win32 | SysPlatform::Cygwin | SysPlatform::Cli => None,
        SysPlatform::Other => Some(LoopType::Uvloop),
    }
}
