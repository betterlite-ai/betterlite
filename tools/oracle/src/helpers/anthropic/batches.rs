use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod cases;
mod input;
mod support;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            input::Input::AuthHeaders(case) => cases::auth_headers(case),
            input::Input::CreateRequests(case) => cases::create_requests(case),
            input::Input::ErrorCounts(case) => cases::error_counts(case),
            input::Input::Lifecycle(case) => cases::lifecycle(case),
            input::Input::ResultResponse(case) => cases::result_response(case),
            input::Input::Retrieve(case) => cases::retrieve(case),
        }
    }
}
