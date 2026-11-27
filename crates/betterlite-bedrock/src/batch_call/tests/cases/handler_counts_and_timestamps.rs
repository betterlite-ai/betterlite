use std::collections::BTreeMap;

use betterlite_openai::batches::records::{BatchObject, RequestCounts};

use crate::batch_call::error::BatchCallError;

use super::super::fixture::{JOB_ARN, REGION, SUBMITTED_EPOCH, lookup_job};

type EndTimes = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

const COUNTS: [(&str, Option<RequestCounts>); 9] = [
    (
        r#""totalRecordCount":10,"successRecordCount":7,"errorRecordCount":3"#,
        Some(RequestCounts {
            completed: 7,
            failed: 3,
            total: 10,
        }),
    ),
    (
        r#""totalRecordCount":2,"successRecordCount":0"#,
        Some(RequestCounts {
            completed: 0,
            failed: 0,
            total: 2,
        }),
    ),
    (r#""totalRecordCount":2"#, None),
    (r#""successRecordCount":2"#, None),
    (r#""totalRecordCount":2,"successRecordCount":1.5"#, None),
    (
        r#""totalRecordCount":2,"successRecordCount":1,"errorRecordCount":"3""#,
        Some(RequestCounts {
            completed: 1,
            failed: 0,
            total: 2,
        }),
    ),
    (
        r#""totalRecordCount":2,"successRecordCount":true"#,
        Some(RequestCounts {
            completed: 1,
            failed: 0,
            total: 2,
        }),
    ),
    (
        r#""totalRecordCount":2,"successRecordCount":1,"errorRecordCount":-1"#,
        Some(RequestCounts {
            completed: 1,
            failed: -1,
            total: 2,
        }),
    ),
    (
        r#""totalRecordCount":true,"successRecordCount":1"#,
        Some(RequestCounts {
            completed: 1,
            failed: 0,
            total: 1,
        }),
    ),
];

const CREATED_AT: [(&str, i64); 6] = [
    (r#""submitTime":"2024-01-15T10:30:00Z""#, SUBMITTED_EPOCH),
    (r#""submitTime":1705314600"#, SUBMITTED_EPOCH),
    ("", 0),
    (r#""submitTime":true"#, 1),
    (r#""submitTime":false"#, 0),
    (r#""submitTime":1.5"#, 1),
];

pub(super) fn handler_counts_and_timestamps() {
    record_counts();
    created_at_sources();
    in_progress_at_sources();
    end_times();
    no_end_time();
    invalid_timestamps();
}

fn job_body(extra: &str) -> String {
    if extra.is_empty() {
        return format!(r#"{{"jobArn":"{JOB_ARN}"}}"#);
    }
    format!(r#"{{"jobArn":"{JOB_ARN}",{extra}}}"#)
}

fn project(extra: &str) -> Result<BatchObject, BatchCallError> {
    let run = lookup_job(&job_body(extra), JOB_ARN, Some(REGION), &BTreeMap::new());
    run.result
}

fn record_counts() {
    for (fragment, expected) in COUNTS {
        let batch = project(fragment).expect("the job projects");
        assert_eq!(batch.request_counts, expected);
    }
}

fn created_at_sources() {
    for (fragment, expected) in CREATED_AT {
        let batch = project(fragment).expect("the job projects");
        assert_eq!(batch.created_at, expected);
    }
}

fn in_progress_at_sources() {
    let cases = [
        (
            r#""lastModifiedTime":"2024-01-15T10:30:00Z""#,
            Some(SUBMITTED_EPOCH),
        ),
        (r#""lastModifiedTime":1705314600"#, Some(SUBMITTED_EPOCH)),
        (r#""lastModifiedTime":null"#, None),
        (r#""lastModifiedTime":0"#, Some(0)),
    ];
    for (fragment, expected) in cases {
        let batch = project(fragment).expect("the job projects");
        assert_eq!(batch.in_progress_at, expected);
    }
}

fn assert_end_times(status: &str, expected: EndTimes) {
    let extra = format!(r#""status":"{status}","endTime":1705314600"#);
    let batch = project(&extra).expect("the job projects");
    let actual = (
        batch.completed_at,
        batch.failed_at,
        batch.cancelled_at,
        batch.expired_at,
    );
    assert_eq!(actual, expected);
}

fn end_times() {
    let at = Some(SUBMITTED_EPOCH);
    assert_end_times("Completed", (at, None, None, None));
    assert_end_times("PartiallyCompleted", (at, None, None, None));
    assert_end_times("Failed", (None, at, None, None));
    assert_end_times("Stopped", (None, None, at, None));
    assert_end_times("Expired", (None, None, None, at));
    assert_end_times("Stopping", (None, None, None, None));
    assert_end_times("InProgress", (None, None, None, None));
}

fn no_end_time() {
    let batch = project(r#""status":"Completed","endTime":null"#).expect("the job projects");
    assert_eq!(batch.completed_at, None);
}

fn invalid_timestamps() {
    let cases = [
        (r#""submitTime":"not-a-time""#, "submitTime"),
        (r#""submitTime":[]"#, "submitTime"),
        (r#""lastModifiedTime":[]"#, "lastModifiedTime"),
        (r#""endTime":"not-a-time""#, "endTime"),
        (r#""endTime":[]"#, "endTime"),
    ];
    for (fragment, field) in cases {
        let error = project(fragment).expect_err("a timestamp outside ISO 8601 is an error");
        assert!(matches!(error, BatchCallError::Field { field: actual } if actual == field));
    }
}
