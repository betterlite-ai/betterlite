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
    let text = match (&scenario.raw, &scenario.payload) {
        (Some(raw), _) => wrapped(scenario.wrap, raw),
        (None, Some(payload)) => {
            let sealed = seal(prefix_of(scenario.prefix), payload, cipher).map_err(failure)?;
            wrapped(scenario.wrap, &sealed)
        }
        (None, None) => return Err(failure("an open scenario needs a payload or raw text")),
    };
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

fn wrapped(wrap: Wrap, text: &str) -> String {
    match wrap {
        Wrap::None => text.to_owned(),
        Wrap::Tamper => tampered(text),
        Wrap::Truncate => text
            .chars()
            .take(text.chars().count().saturating_sub(1))
            .collect(),
        Wrap::SwapPrefix => text.strip_prefix(GATEWAY_DCR_CLIENT_ID_PREFIX).map_or_else(
            || text.to_owned(),
            |ciphertext| format!("{GATEWAY_AUTH_CODE_PREFIX}{ciphertext}"),
        ),
    }
}
const fn prefix_of(prefix: SealPrefix) -> &'static str {
    match prefix {
        SealPrefix::Client => GATEWAY_DCR_CLIENT_ID_PREFIX,
        SealPrefix::Code => GATEWAY_AUTH_CODE_PREFIX,
        SealPrefix::Bare => "",
    }
}
fn tampered(text: &str) -> String {
    let mut characters: Vec<char> = text.chars().collect();
    let middle = characters.len().div_euclid(2);
    if let Some(character) = characters.get_mut(middle) {
        *character = if *character == 'A' { 'B' } else { 'A' };
    }
    characters.into_iter().collect()
}
