#![cfg(test)]

use indexmap::IndexMap;

use super::Prepared;
use crate::realtime_calls::convert::{
    CallsArguments, ClientSecretArguments, Common, TranscriptionArguments,
};

fn common() -> Common {
    Common {
        model: None,
        timeout: None,
        api_base: None,
        api_key: None,
        api_version: None,
        extra_headers: None,
        kwargs: IndexMap::new(),
    }
}

#[test]
fn prepared_call_type_names_the_litellm_call() {
    let secret = Prepared::ClientSecret(ClientSecretArguments {
        common: common(),
        session: None,
        expires_after: None,
    });
    let transcription = Prepared::Transcription(TranscriptionArguments {
        common: common(),
        transcription_session: None,
    });
    let calls = Prepared::Calls(CallsArguments {
        common: common(),
        ephemeral_key: String::from("ek"),
        sdp: Vec::new(),
        session: None,
    });
    assert_eq!(secret.call_type(), "arealtime_client_secret");
    assert_eq!(transcription.call_type(), "arealtime_transcription_session");
    assert_eq!(calls.call_type(), "arealtime_calls");
}
