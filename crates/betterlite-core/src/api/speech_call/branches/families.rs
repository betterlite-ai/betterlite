use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use secrecy::ExposeSecret as _;
use serde_json::Value;

use super::super::operations::SpeechOperationKind;
use super::super::service::BranchCall;
use super::super::values;

// the provider families whose branch is the shared skeleton with one of these adjustments
#[derive(Clone, Copy)]
pub(crate) enum Rules {
    Edenai,
    Minimax,
    Mistral,
    RunwayMl,
    Polly,
    AzureSpeech,
}

pub(super) enum VoiceRule {
    Keep,
    String,
    FirstOf(&'static [&'static str]),
}

// the providers that run the shared generic branch
pub(crate) fn rules(provider: &str) -> Option<Rules> {
    match provider {
        "edenai" => Some(Rules::Edenai),
        "minimax" => Some(Rules::Minimax),
        "mistral" => Some(Rules::Mistral),
        "runwayml" => Some(Rules::RunwayMl),
        "aws_polly" => Some(Rules::Polly),
        _ => None,
    }
}

// every family shares one branch shape and differs only in these five answers
pub(super) const fn kind(family: Rules) -> SpeechOperationKind {
    match family {
        Rules::Edenai | Rules::Minimax | Rules::Mistral => SpeechOperationKind::Generic,
        Rules::RunwayMl | Rules::Polly | Rules::AzureSpeech => SpeechOperationKind::Dispatch,
    }
}

pub(super) const fn voice_rule(family: Rules) -> VoiceRule {
    match family {
        Rules::Edenai | Rules::Mistral => VoiceRule::String,
        Rules::Minimax => VoiceRule::FirstOf(&["voice_id", "id", "name"]),
        Rules::RunwayMl | Rules::Polly | Rules::AzureSpeech => VoiceRule::Keep,
    }
}

pub(super) const fn overwrites_credentials(family: Rules) -> bool {
    match family {
        Rules::Minimax | Rules::Mistral => true,
        Rules::Edenai | Rules::RunwayMl | Rules::Polly | Rules::AzureSpeech => false,
    }
}

pub(super) const fn carries_base(family: Rules) -> bool {
    match family {
        Rules::Edenai => true,
        Rules::Minimax | Rules::Mistral | Rules::RunwayMl | Rules::Polly | Rules::AzureSpeech => {
            false
        }
    }
}

pub(super) const fn required_config(family: Rules) -> Option<&'static str> {
    match family {
        Rules::RunwayMl => Some("RunwayML Text-to-Speech configuration not found"),
        Rules::AzureSpeech => Some("Azure Speech Service configuration not found"),
        Rules::Edenai | Rules::Minimax | Rules::Mistral | Rules::Polly => None,
    }
}

// the generic handler takes the voice only when it is a string
pub(super) fn string_voice(voice: Option<&SpeechVoice>) -> Option<SpeechVoice> {
    match voice {
        Some(SpeechVoice::Name(name)) => Some(SpeechVoice::Name(name.clone())),
        Some(SpeechVoice::Options(_)) | None => None,
    }
}

// MiniMax reads the first non-empty of voice_id, id and name from a dict voice
pub(super) fn first_voice(voice: Option<&SpeechVoice>, names: &[&str]) -> Option<SpeechVoice> {
    match voice {
        Some(SpeechVoice::Name(name)) => Some(SpeechVoice::Name(name.clone())),
        Some(SpeechVoice::Options(options)) => {
            values::first_text(options, names).map(|id| SpeechVoice::Name(id.to_owned()))
        }
        None => None,
    }
}

// an explicit non-None base or key overwrites the litellm_params entry, empty strings included
pub(super) fn overwrite_credentials(params: &mut Object, branch: &BranchCall<'_, '_>) {
    if let Some(api_base) = &branch.resolved.api_base {
        params.insert(String::from("api_base"), Value::String(api_base.clone()));
    }
    if let Some(api_key) = &branch.call.api_key {
        params.insert(
            String::from("api_key"),
            Value::String(api_key.expose_secret().to_owned()),
        );
    }
}
