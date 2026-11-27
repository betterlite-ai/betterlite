use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::api::batch_enqueued_reserve::BatchEnqueuedTokenScope;
use crate::api::batch_limit_counters::records::BatchFileUsage;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::request_destination::RequestData;

use super::BatchPrecallError;
use super::fields;
use super::input::BatchCreateCall;
use super::ports::{BatchAccountRequest, BatchPrecallSkip, BatchPrecallStages};
use super::result::{BatchCreateOutcome, BatchCreateResult, BatchDiagnostic, debug, error};

const CREATE_CALL_TYPE: &str = "acreate_batch";

pub(crate) struct CreateHook<'a, Stages> {
    pub(crate) stages: &'a Stages,
}

impl<Stages: BatchPrecallStages> CreateHook<'_, Stages> {
    pub(crate) async fn run(
        &self,
        call: &BatchCreateCall,
    ) -> Result<BatchCreateResult, BatchPrecallError> {
        let mut diagnostics = Vec::new();
        let mut data = call.data.clone();
        if call.call_type != CREATE_CALL_TYPE {
            let message = format!(
                "Batch rate limiter: Not handling batch creation rate limiting for call type: {}",
                call.call_type
            );
            diagnostics.push(debug(message));
            return Ok(passed(data, diagnostics));
        }
        diagnostics.push(debug(String::from(
            "Batch rate limiter: Handling batch creation rate limiting",
        )));
        let Some(file_id) = fields::input_file_id(&data)?.filter(|value| !value.is_empty()) else {
            diagnostics.push(debug(String::from(
                "No input_file_id in batch request, skipping rate limiting",
            )));
            return Ok(passed(data, diagnostics));
        };
        let file_id = file_id.to_owned();
        let scopes = fields::enqueued_scopes(&call.principal)?;
        let attempt = CreateAttempt {
            principal: &call.principal,
            file_id: &file_id,
            scopes: &scopes,
            data: &mut data,
            diagnostics: &mut diagnostics,
        };
        match attempt.run(self.stages).await {
            Ok(outcome) => Ok(BatchCreateResult {
                data,
                outcome,
                diagnostics,
            }),
            Err(AttemptFailure::Field(failure)) => Err(failure),
            Err(AttemptFailure::Stage(failure)) => Ok(self.failed(data, diagnostics, &failure)),
        }
    }

    // except HTTPException: raise leaves the hook as an HTTP error; every other failure is logged and the request goes on
    fn failed(
        &self,
        data: RequestData,
        mut diagnostics: Vec<BatchDiagnostic>,
        failure: &Stages::Error,
    ) -> BatchCreateResult {
        if let Some(http) = self.stages.http(failure) {
            return BatchCreateResult {
                data,
                outcome: BatchCreateOutcome::Raised(http.clone()),
                diagnostics,
            };
        }
        diagnostics.push(error(format!("Error in batch rate limiting: {failure}")));
        passed(data, diagnostics)
    }
}

// a stage that failed, apart from a request field the typed model cannot hold: betterlite names the field where the source lets it through
enum AttemptFailure<StagesError> {
    Field(BatchPrecallError),
    Stage(StagesError),
}

struct CreateAttempt<'a> {
    principal: &'a BatchLimitPrincipal,
    file_id: &'a str,
    scopes: &'a [BatchEnqueuedTokenScope],
    data: &'a mut RequestData,
    diagnostics: &'a mut Vec<BatchDiagnostic>,
}

impl CreateAttempt<'_> {
    async fn run<Stages: BatchPrecallStages>(
        mut self,
        stages: &Stages,
    ) -> Result<BatchCreateOutcome, AttemptFailure<Stages::Error>> {
        let decision = stages
            .should_skip(self.principal, self.data, !self.scopes.is_empty())
            .map_err(AttemptFailure::Stage)?;
        if decision.skip {
            return Ok(BatchCreateOutcome::Passed);
        }
        let usage = self.record_usage(stages, &decision).await?;
        self.settle(stages, &decision, &usage).await
    }

    // count_input_file_usage, then the two counters the route reads afterwards
    async fn record_usage<Stages: BatchPrecallStages>(
        &mut self,
        stages: &Stages,
        decision: &BatchPrecallSkip,
    ) -> Result<BatchFileUsage, AttemptFailure<Stages::Error>> {
        // the source reads the provider only once the file will be processed
        let custom_llm_provider =
            fields::custom_llm_provider(self.data).map_err(AttemptFailure::Field)?;
        self.diagnostics.push(debug(format!(
            "Counting tokens from batch input file: {}",
            self.file_id
        )));
        let request = BatchAccountRequest {
            file_id: self.file_id,
            custom_llm_provider,
            data: self.data,
            descriptors: decision.descriptors.as_deref(),
        };
        let usage = stages
            .account(self.principal, &request)
            .await
            .map_err(AttemptFailure::Stage)?;
        self.diagnostics.push(debug(format!(
            "Batch input file usage - Tokens: {}, Requests: {}",
            usage.total_tokens, usage.request_count
        )));
        fields::insert_usage(self.data, &usage);
        Ok(usage)
    }

    // enqueued scopes reserve, a plain key charges, and a refusal returns without a success line
    async fn settle<Stages: BatchPrecallStages>(
        &mut self,
        stages: &Stages,
        decision: &BatchPrecallSkip,
        usage: &BatchFileUsage,
    ) -> Result<BatchCreateOutcome, AttemptFailure<Stages::Error>> {
        if self.scopes.is_empty() {
            let rejection = stages
                .charge(
                    self.principal,
                    self.data,
                    usage,
                    decision.descriptors.as_deref(),
                )
                .await
                .map_err(AttemptFailure::Stage)?;
            if rejection.is_none() {
                self.diagnostics.push(debug(String::from(
                    "Batch rate limit check passed, counters incremented",
                )));
            }
            return Ok(judge(rejection));
        }
        let rejection = stages
            .reserve(self.principal, self.data, usage, self.scopes)
            .await
            .map_err(AttemptFailure::Stage)?;
        if rejection.is_none() {
            self.diagnostics.push(debug(String::from(
                "Batch enqueued-token reservation succeeded",
            )));
        }
        Ok(judge(rejection))
    }
}

fn judge(rejection: Option<Box<ProxyRateLimitError>>) -> BatchCreateOutcome {
    rejection.map_or(BatchCreateOutcome::Passed, BatchCreateOutcome::Rejected)
}

fn passed(data: RequestData, diagnostics: Vec<BatchDiagnostic>) -> BatchCreateResult {
    BatchCreateResult {
        data,
        outcome: BatchCreateOutcome::Passed,
        diagnostics,
    }
}
