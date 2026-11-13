#![cfg(test)]

use betterlite_config::sections::io_token_reservations as config;
use betterlite_router::io_token_reservations::limits::IoLimits;

use super::deployment_limits;

#[test]
fn deployment_limits_stay_index_aligned_and_keep_zero() {
    let section = config::Settings {
        deployments: vec![
            config::DeploymentIoLimits {
                itpm: Some(0),
                otpm: None,
            },
            config::DeploymentIoLimits {
                itpm: None,
                otpm: Some(2),
            },
            config::DeploymentIoLimits {
                itpm: Some(3),
                otpm: Some(4),
            },
        ]
        .into_boxed_slice(),
    };
    assert_eq!(
        deployment_limits(&section),
        vec![
            IoLimits {
                itpm: Some(0),
                otpm: None,
            },
            IoLimits {
                itpm: None,
                otpm: Some(2),
            },
            IoLimits {
                itpm: Some(3),
                otpm: Some(4),
            },
        ]
    );
}

#[test]
fn an_empty_section_has_no_deployment_limits() {
    assert!(deployment_limits(&config::Settings::default()).is_empty());
}
