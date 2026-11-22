use crate::api::file_batch_guardrails::records::BatchScanFailure;

// One record's problem: the refusal the scan reached, or the exception the guardrail chain raised
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanProblem<Raised> {
    Failure(BatchScanFailure),
    Raised(Raised),
}

// _worst: a raised exception outranks a refusal, then the earliest line wins; None when nothing failed
#[must_use]
pub fn worst_problem<Raised>(problems: &[(usize, ScanProblem<Raised>)]) -> Option<usize> {
    problems
        .iter()
        .enumerate()
        .min_by_key(|(_, (line_number, problem))| {
            (matches!(problem, ScanProblem::Failure(_)), *line_number)
        })
        .map(|(index, _)| index)
}
