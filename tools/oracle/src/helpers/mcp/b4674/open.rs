use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_dcr::contracts::{
    GATEWAY_AUTH_CODE_PREFIX, GATEWAY_DCR_CLIENT_ID_PREFIX,
};
use betterlite_mcp::gateway_dcr::sealed::{open_gateway_dcr_client, seal};
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::{Value, json};

use super::{OpenScenario, SealPrefix, Wrap, failure};

pub(super) fn reports(
    scenarios: &[OpenScenario],
    cipher: &DbCipher,
) -> Result<Vec<Value>, Box<HelperError>> {
    scenarios
        .iter()
        .map(|scenario| report(scenario, cipher))
        .collect()
}

fn report(scenario: &OpenScenario, cipher: &DbCipher) -> Result<Value, Box<HelperError>> {
    let text = wrapped_text(scenario, cipher)?;
    Ok(match open_gateway_dcr_client(&text, cipher) {
        Some(client) => json!({
            "name": scenario.name,
            "opened": true,
            "redirect_uris": client.redirect_uris,
            "iat": client.iat,
        }),
        None => json!({
            "name": scenario.name,
            "opened": false,
            "redirect_uris": null,
            "iat": null,
        }),
    })
}

fn wrapped_text(scenario: &OpenScenario, cipher: &DbCipher) -> Result<String, Box<HelperError>> {
    let text = raw_text(scenario, cipher)?;
    Ok(match scenario.wrap {
        Wrap::None => text,
        Wrap::Tamper => tampered(&text),
        Wrap::Truncate => text
            .chars()
            .take(text.chars().count().saturating_sub(1))
            .collect(),
        Wrap::SwapPrefix => match text.strip_prefix(GATEWAY_DCR_CLIENT_ID_PREFIX) {
            Some(ciphertext) => format!("{GATEWAY_AUTH_CODE_PREFIX}{ciphertext}"),
            None => text,
        },
    })
}

fn raw_text(scenario: &OpenScenario, cipher: &DbCipher) -> Result<String, Box<HelperError>> {
    if let Some(raw) = &scenario.raw {
        return Ok(raw.clone());
    }
    let Some(payload) = &scenario.payload else {
        return Err(failure("an open scenario needs a payload or raw text"));
    };
    seal(prefix_of(scenario.prefix), payload, cipher).map_err(failure)
}

const fn prefix_of(prefix: SealPrefix) -> &'static str {
    match prefix {
        SealPrefix::Client => GATEWAY_DCR_CLIENT_ID_PREFIX,
        SealPrefix::Code => GATEWAY_AUTH_CODE_PREFIX,
        SealPrefix::Bare => "",
    }
}

fn tampered(text: &str) -> String {
    let middle = text.chars().count() / 2;
    text.chars()
        .enumerate()
        .map(|(index, character)| {
            if index != middle {
                character
            } else if character == 'A' {
                'B'
            } else {
                'A'
            }
        })
        .collect()
}
