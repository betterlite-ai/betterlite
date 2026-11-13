use crate::startup_input::decode_worker_config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelPlan {
    pub package: Option<LogLevel>,
    pub router: Option<LogLevel>,
    pub proxy: Option<LogLevel>,
}

const EMPTY_PLAN: LevelPlan = LevelPlan {
    package: None,
    router: None,
    proxy: None,
};

const ALL_INFO: LevelPlan = LevelPlan {
    package: Some(LogLevel::Info),
    router: Some(LogLevel::Info),
    proxy: Some(LogLevel::Info),
};

const ALL_DEBUG: LevelPlan = LevelPlan {
    package: Some(LogLevel::Debug),
    router: Some(LogLevel::Debug),
    proxy: Some(LogLevel::Debug),
};

const ROUTER_PROXY_INFO: LevelPlan = LevelPlan {
    package: None,
    router: Some(LogLevel::Info),
    proxy: Some(LogLevel::Info),
};

const ROUTER_PROXY_DEBUG: LevelPlan = LevelPlan {
    package: None,
    router: Some(LogLevel::Debug),
    proxy: Some(LogLevel::Debug),
};

#[must_use]
pub fn plan_verbose_levels(
    worker_config: Option<&str>,
    worker_config_is_file: bool,
    litellm_log: &str,
    warn: &mut dyn FnMut(&str),
) -> LevelPlan {
    let Some(raw) = worker_config else {
        return EMPTY_PLAN;
    };
    if worker_config_is_file {
        return EMPTY_PLAN;
    }
    let flags = match decode_worker_config(raw) {
        Ok(flags) => flags,
        Err(error) => {
            warn(&format!("Failed to init verbose loggers: {error}"));
            return EMPTY_PLAN;
        }
    };
    // detailed_debug replaces the Info that debug set a step earlier
    if flags.detailed_debug == Some(true) {
        return ALL_DEBUG;
    }
    if flags.debug == Some(true) {
        return ALL_INFO;
    }
    if flags.debug == Some(false) && flags.detailed_debug == Some(false) {
        if litellm_log.eq_ignore_ascii_case("INFO") {
            return ROUTER_PROXY_INFO;
        }
        if litellm_log.eq_ignore_ascii_case("DEBUG") {
            return ROUTER_PROXY_DEBUG;
        }
    }
    EMPTY_PLAN
}

mod tests;
