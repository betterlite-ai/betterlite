#![cfg(test)]

use betterlite_config::sections::cooldown_state as config;
use betterlite_router::cooldown_state::settings::Settings;

use super::cooldown_settings;

#[test]
fn the_router_settings_carry_every_configured_key() {
    let section = config::Settings {
        enable_health_check_routing: true,
        staleness_threshold: 90.0,
        ignore_transient_errors: true,
        read_interval_seconds: 0.25,
    };
    assert_eq!(
        cooldown_settings(&section, 12.0),
        Settings {
            default_cooldown_time: 12.0,
            redis_read_interval_seconds: 0.25,
            enable_health_check_routing: true,
            staleness_threshold: 90.0,
            ignore_transient_errors: true,
        }
    );
}

#[test]
fn an_empty_section_keeps_the_router_defaults() {
    let section = config::Settings::default();
    assert_eq!(cooldown_settings(&section, 5.0), Settings::default());
}
