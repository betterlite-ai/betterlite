#![cfg(test)]

use super::{
    AgenticLoopCeilingError, AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS,
    validated_max_agentic_loops,
};

#[test]
fn agentic_loop_ceiling_accepts_whole_scalars() {
    assert_eq!(
        validated_max_agentic_loops(&AgenticLoopValue::Integer(5), "field"),
        Ok(Some(5))
    );
    assert_eq!(
        validated_max_agentic_loops(&AgenticLoopValue::Float(5.0), "field"),
        Ok(Some(5))
    );
    assert_eq!(
        validated_max_agentic_loops(&AgenticLoopValue::Text(" 5 ".to_owned()), "field"),
        Ok(Some(5))
    );
}

#[test]
fn agentic_loop_ceiling_rejects_bool_fraction_and_bad_text() {
    for value in [
        AgenticLoopValue::Boolean(true),
        AgenticLoopValue::Float(1.5),
        AgenticLoopValue::Text("five".to_owned()),
    ] {
        assert!(matches!(
            validated_max_agentic_loops(&value, "litellm_settings.max_agentic_loops"),
            Err(AgenticLoopCeilingError::NotWholeNumber { field })
                if field == "litellm_settings.max_agentic_loops"
        ));
    }
}

#[test]
fn agentic_loop_ceiling_rejects_below_one_and_defaults() {
    assert!(matches!(
        validated_max_agentic_loops(&AgenticLoopValue::Integer(0), "field"),
        Err(AgenticLoopCeilingError::BelowMinimum { field, value: 0 })
            if field == "field"
    ));
    assert_eq!(
        validated_max_agentic_loops(&AgenticLoopValue::Null, "field"),
        Ok(None)
    );
    assert_eq!(DEFAULT_MAX_AGENTIC_LOOPS, 3);
}
