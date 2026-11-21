use crate::rejected_fields::{
    FoundryRepair, INVALID_TOOL_CALL_INDEX_MARKER, MAX_ATTEMPTS, PASS_THROUGH_HINT, repair,
};

use super::support;

pub(super) fn drop_top_level_loc() {
    let repaired = repair(
        &support::extra_inputs("temperature"),
        &support::body(support::CHAT),
    )
    .expect("the repair runs");
    assert_eq!(repaired, support::body(support::CHAT_WITHOUT_TEMPERATURE));
    // a message written as an object carries the same detail
    let object_message = serde_json::json!({
        "error": {"message": {"detail": [{"loc": ["body", "temperature"]}]}},
    })
    .to_string();
    let repaired = repair(&object_message, &support::body(support::CHAT)).expect("the repair runs");
    assert_eq!(repaired, support::body(support::CHAT_WITHOUT_TEMPERATURE));
}

pub(super) fn drop_tool_level_extra_fields() {
    let request = support::body(
        r#"{"model":"deployment","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"first"},"extra":1},{"type":"function","function":{"name":"second"},"other-param":true},{"type":"function","function":{"name":"third"},"extra":3},"plain-text"],"tool_choice":"auto"}"#,
    );
    let repaired = repair(
        &support::wrapped(
            "Extra inputs are not permitted: tools[0].extra and tools[1].other-param are not allowed",
        ),
        &request,
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(
            r#"{"model":"deployment","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"first"}},{"type":"function","function":{"name":"second"}},{"type":"function","function":{"name":"third"}},"plain-text"],"tool_choice":"auto"}"#,
        )
    );
}

pub(super) fn remove_tool_call_index() {
    let both = support::wrapped(&format!(
        "{INVALID_TOOL_CALL_INDEX_MARKER}. {PASS_THROUGH_HINT} ['temperature', 'top_p']"
    ));
    let request = support::body(
        r#"{"model":"deployment","temperature":2,"top_p":0.5,"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"f","arguments":"{}"},"index":0}]},{"role":"tool","tool_call_id":"call-1","content":"done"}]}"#,
    );
    let repaired = repair(&both, &request).expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(
            r#"{"model":"deployment","temperature":2,"top_p":0.5,"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call-1","content":"done"}]}"#,
        )
    );
    // with messages null the invalid-index branch is skipped and the pass-through elif runs
    let repaired = repair(
        &both,
        &support::body(r#"{"model":"deployment","temperature":2,"top_p":0.5,"messages":null}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","messages":null}"#)
    );
    // an absent messages key skips it the same way
    let repaired = repair(
        &both,
        &support::body(r#"{"model":"deployment","temperature":2,"top_p":0.5}"#),
    )
    .expect("the repair runs");
    assert_eq!(repaired, support::body(r#"{"model":"deployment"}"#));
}

pub(super) fn extra_parameters_list() {
    let request = support::body(
        r#"{"model":"deployment","stream_options":{"include_usage":true},"extra-parameters":"error","top_p":0.5}"#,
    );
    let repaired = repair(
        &support::pass_through(
            "Extra parameters ['stream_options', 'extra-parameters'] are not allowed when extra-parameters is not set or set to be 'error'.",
        ),
        &request,
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","top_p":0.5}"#)
    );
    // only the first bracket pair is read
    let repaired = repair(
        &support::pass_through("Extra parameters [top_p] are not allowed. Also [temperature]."),
        &support::body(r#"{"model":"deployment","top_p":0.5,"temperature":2,"n":2}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","temperature":2,"n":2}"#)
    );
    // no bracket names nothing
    let repaired = repair(
        &support::pass_through("Extra parameters are not allowed."),
        &support::body(r#"{"model":"deployment","top_p":0.5}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","top_p":0.5}"#)
    );
    // a double quote inside a JSON message keeps its backslash, so only the single-quoted name strips cleanly
    let repaired = repair(
        &support::pass_through("Extra parameters [ \"n\" , 'top_p' ,, ] are not allowed."),
        &support::body(r#"{"model":"deployment","n":2,"top_p":0.5,"temperature":2}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","n":2,"temperature":2}"#)
    );
    // quotes that are JSON syntax rather than string content strip cleanly
    let sibling = serde_json::json!({
        "error": {"message": PASS_THROUGH_HINT},
        "params": ["n", "top_p"],
    })
    .to_string();
    let repaired = repair(
        &sibling,
        &support::body(r#"{"model":"deployment","n":2,"top_p":0.5,"temperature":2}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","temperature":2}"#)
    );
    // a bracket still open at the end of its line is skipped by the dot in the pattern
    let multiline = "{\n  \"error\": {\"message\": \"Set extra-parameters to 'pass-through'.\"},\n  \"fields\": [\n    \"top_p\"\n  ]\n}";
    let repaired = repair(
        multiline,
        &support::body(r#"{"model":"deployment","top_p":0.5}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","top_p":0.5}"#)
    );
    // an empty bracket pair names nothing
    let repaired = repair(
        &support::pass_through("Extra parameters [] are not allowed."),
        &support::body(r#"{"model":"deployment","top_p":0.5}"#),
    )
    .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(r#"{"model":"deployment","top_p":0.5}"#)
    );
}

pub(super) fn repair_without_drop_policy_for_tools() {
    let error = support::wrapped("Extra inputs are not permitted: tools[0].extra");
    assert!(
        FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&error)
    );
    let repaired = repair(&error, &support::body(support::TOOLS)).expect("the repair runs");
    assert_eq!(repaired, support::body(support::TOOLS_WITHOUT_EXTRA));
}

pub(super) fn unrelated_error_no_retry() {
    let error = support::wrapped("a field the model does not know");
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&error)
    );
    assert!(
        !FoundryRepair {
            call: true,
            global: true,
        }
        .should_retry(&error)
    );
    // the marker alone is not enough without a drop policy or a tool-level field
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped("Extra inputs are not permitted"))
    );
}

pub(super) fn two_repairs_only() {
    assert_eq!(MAX_ATTEMPTS, 2_u32);
    let policy = FoundryRepair {
        call: true,
        global: false,
    };
    let error = support::extra_inputs("temperature");
    assert!(policy.should_retry(&error));
    let once = repair(&error, &support::body(support::CHAT)).expect("the repair runs");
    let twice = repair(&error, &once).expect("the repair runs");
    assert_eq!(twice, once);
    // the repaired body lost the field, but the same error text still asks for a retry, so the bound ends the loop
    assert!(policy.should_retry(&error));
}

pub(super) fn repair_trigger_literals() {
    let plain = support::wrapped("Extra inputs are not permitted");
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&plain)
    );
    assert!(
        FoundryRepair {
            call: true,
            global: false,
        }
        .should_retry(&plain)
    );
    assert!(
        FoundryRepair {
            call: false,
            global: true,
        }
        .should_retry(&plain)
    );
    assert!(
        FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped(
            "Extra inputs are not permitted tools[12].x"
        ))
    );
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped(
            "Extra inputs are not permitted tools[x].x"
        ))
    );
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped(
            "Extra inputs are not permitted tools[0]x"
        ))
    );
    assert!(
        FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped(INVALID_TOOL_CALL_INDEX_MARKER))
    );
    assert!(
        FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped(PASS_THROUGH_HINT))
    );
    assert!(
        FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped(&format!(
            "{INVALID_TOOL_CALL_INDEX_MARKER} {PASS_THROUGH_HINT}"
        )))
    );
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped("extra inputs are not permitted"))
    );
    assert!(
        !FoundryRepair {
            call: false,
            global: false,
        }
        .should_retry(&support::wrapped("a field the model does not know"))
    );
}
