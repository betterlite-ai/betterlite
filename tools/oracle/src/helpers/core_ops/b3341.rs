mod input;
mod record;

use betterlite_core::api::video_character_calls::error::CharacterCallFailure;
use betterlite_core::api::video_character_calls::{
    CharacterCallDeps, CharacterOutcome, avideo_create_character, avideo_get_character,
    video_create_character, video_get_character,
};
use betterlite_core::api::video_records::CharacterObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::input::{Function, Input};
use self::record::{LoggingView, Output, Recorder};

// B198's DEFAULT_REQUEST_TIMEOUT with REQUEST_TIMEOUT unset, the value the cases run under
const REQUEST_TIMEOUT_SECONDS: f64 = 6000.0;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(answer(input))
    }
}

async fn answer(input: &Input) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mut recorder = Recorder::new(input.answered_object())?;
    let deps = CharacterCallDeps {
        redactor: &redactor,
        request_timeout_seconds: REQUEST_TIMEOUT_SECONDS,
    };
    let character = match (input.function, input.asynchronous) {
        (Function::Create, false) => direct(video_create_character(
            &mut recorder,
            deps,
            &input.create_call(),
        ))?,
        (Function::Create, true) => {
            avideo_create_character(&mut recorder, deps, &input.create_call())
                .await
                .map_err(|error| helper_failure(error.to_string()))?
        }
        (Function::Get, false) => {
            direct(video_get_character(&mut recorder, deps, &input.get_call()))?
        }
        (Function::Get, true) => avideo_get_character(&mut recorder, deps, &input.get_call())
            .await
            .map_err(|error| helper_failure(error.to_string()))?,
    };
    let output = Output {
        result: character,
        calls: recorder.calls,
        logging: LoggingView::of(&recorder.logging),
    };
    serde_json::to_value(output).or_value_error()
}

// the sync answer is in hand, and a future here breaks the blocking entry's contract
fn direct(
    outcome: Result<CharacterOutcome<'_>, CharacterCallFailure>,
) -> Result<CharacterObject, Box<HelperError>> {
    match outcome.map_err(|error| helper_failure(error.to_string()))? {
        CharacterOutcome::Done(character) => Ok(character),
        CharacterOutcome::Pending(_) => Err(helper_failure(
            "the blocking character handler answered with a pending future".to_owned(),
        )),
    }
}

fn helper_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "CharacterCallFailure".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
