import json
from typing import TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite import _native

NOTE = "This may be a false positive."
LONG_VALUE = "x" * 512
EMPTY = "{}"
NAMED_GUARDRAIL = '{"guardrails": ["pii"]}'
SCORE = 1.5
CALL_TIMEOUT = 12.5
UPDATED_TIMEOUT = 4.5
SETTINGS: dict[str, JsonValue] = {
    "guardrail_name": "pii",
    "supported_event_hooks": ["pre_call", "post_call"],
    "event_hook": "pre_call",
}


class ErrorPayload(TypedDict):
    kind: str
    message: str
    guardrail_name: str | None


class SettingsView(TypedDict):
    guardrail_name: str | None
    supported_event_hooks: JsonValue
    event_hook: JsonValue
    default_on: bool
    mask_request_content: bool
    mask_response_content: bool
    violation_message_template: str | None
    sticky_session_routing: bool
    run_in_parallel: bool
    timeout: float | None
    switches: dict[str, bool]


class Record(TypedDict):
    guardrail_name: str | None
    guardrail_mode: JsonValue
    guardrail_response: JsonValue
    guardrail_status: str
    start_time: float | None
    duration: float | None


class TracedRecord(Record, total=False):
    guardrail_id: str
    guardrail_cost: float


class Recorded(TypedDict):
    metadata: dict[str, JsonValue]


class AdvisoryData(TypedDict, total=False):
    messages: list[JsonValue]
    input: JsonValue


class Params(TypedDict):
    metadata: dict[str, JsonValue]


class LogTarget(TypedDict):
    litellm_params: Params
    model_call_params: Params | None


class Sanitized(TypedDict, total=False):
    match: JsonValue
    secret_fields: JsonValue
    score: float
    outer: dict[str, JsonValue]


def build(settings: dict[str, JsonValue], *, strict: bool = True) -> _native.NativeGuardrail:
    text = json.dumps(settings)
    return _native.NativeGuardrail(text, strict)


def record_context(hook: str = "pre_call") -> dict[str, JsonValue]:
    return {
        "settings": {"guardrail_name": "pii", "event_hook": hook},
        "log_target": {"litellm_params": {"metadata": {}}, "model_call_params": None},
    }


def first_record(data_json: str) -> TracedRecord:
    data = TypeAdapter(Recorded).validate_json(data_json)
    entries = TypeAdapter(list[TracedRecord]).validate_python(
        data["metadata"]["standard_logging_guardrail_information"]
    )
    assert len(entries) == 1
    return entries[0]


def test_native_base_settings() -> None:
    settings: dict[str, JsonValue] = {
        "guardrail_name": "pii",
        "supported_event_hooks": ["pre_call", "post_call"],
        "event_hook": ["pre_call", "post_call"],
        "default_on": True,
        "mask_request_content": True,
        "mask_response_content": True,
        "violation_message_template": "blocked: {default_message}",
        "sticky_session_routing": False,
        "timeout": CALL_TIMEOUT,
        "switches": {"deny_text_on_error": True},
    }
    view = TypeAdapter(SettingsView).validate_json(build(settings).settings())
    assert view["guardrail_name"] == "pii"
    assert view["supported_event_hooks"] == ["pre_call", "post_call"]
    assert view["event_hook"] == ["pre_call", "post_call"]
    assert view["default_on"] is True
    assert view["mask_request_content"] is True
    assert view["mask_response_content"] is True
    assert view["violation_message_template"] == "blocked: {default_message}"
    assert view["sticky_session_routing"] is False
    assert view["timeout"] == CALL_TIMEOUT
    assert view["switches"]["deny_text_on_error"] is True
    defaults = TypeAdapter(SettingsView).validate_json(build({"guardrail_name": "pii"}).settings())
    assert defaults["event_hook"] is None
    assert defaults["default_on"] is False
    assert defaults["mask_request_content"] is False
    assert defaults["sticky_session_routing"] is True
    assert defaults["timeout"] is None


def test_native_base_bad_json() -> None:
    with pytest.raises(_native.NativeGuardrailError) as raised:
        _native.NativeGuardrail("{", strict_modes=True)
    payload = TypeAdapter(ErrorPayload).validate_json(str(raised.value))
    assert payload["kind"] == "InvalidJson"
    assert payload["message"]
    assert payload["guardrail_name"] is None


def test_native_base_bad_hook() -> None:
    settings: dict[str, JsonValue] = {
        "guardrail_name": "pii",
        "supported_event_hooks": ["pre_call"],
        "event_hook": "post_call",
    }
    with pytest.raises(_native.NativeGuardrailError) as raised:
        build(settings)
    payload = TypeAdapter(ErrorPayload).validate_json(str(raised.value))
    assert payload["kind"] == "UnsupportedEventHook"
    assert "post_call" in payload["message"]
    assert build(settings, strict=False).settings()


def test_native_base_update() -> None:
    guardrail = build(SETTINGS)
    params: dict[str, JsonValue] = {
        "guardrail": "pii",
        "mode": "post_call",
        "default_on": True,
        "mask_request_content": True,
        "violation_message_template": "Stop: {default_message}",
        "timeout": UPDATED_TIMEOUT,
    }
    guardrail.update_params(json.dumps(params))
    view = TypeAdapter(SettingsView).validate_json(guardrail.settings())
    assert view["default_on"] is True
    assert view["mask_request_content"] is True
    assert view["violation_message_template"] == "Stop: {default_message}"
    assert view["timeout"] == UPDATED_TIMEOUT
    assert view["event_hook"] == "pre_call"
    assert TypeAdapter(str).validate_json(guardrail.mode()) == "post_call"
    assert TypeAdapter(str | None).validate_json(build(SETTINGS).mode()) is None


def test_native_base_violation() -> None:
    templated = build({"guardrail_name": "pii", "violation_message_template": "Blocked: {reason}"})
    assert templated.render_violation_message("nope", '{"reason": "pii"}') == "Blocked: pii"
    default_filled = build({"violation_message_template": "Blocked: {default_message}"})
    assert default_filled.render_violation_message("nope", "{}") == "Blocked: nope"
    assert templated.render_violation_message("nope", '{"other": "x"}') == "nope"
    assert build({"guardrail_name": "pii"}).render_violation_message("nope", "{}") == "nope"


def test_native_base_advisory() -> None:
    guardrail = build(SETTINGS)
    body: dict[str, JsonValue] = {"messages": [{"role": "user", "content": "hi"}]}
    written, text = guardrail.inject_advisory_message(json.dumps(body), NOTE)
    assert written is True
    data = TypeAdapter(AdvisoryData).validate_json(text)
    assert data.get("messages") == [
        {"role": "user", "content": "hi"},
        {"role": "system", "content": NOTE},
    ]
    structured: dict[str, JsonValue] = {"input": [{"role": "user", "content": "hi"}]}
    written, text = guardrail.inject_advisory_message(json.dumps(structured), NOTE)
    assert written is False
    assert TypeAdapter(AdvisoryData).validate_json(text) == structured


def test_native_select_hooks() -> None:
    pre = build({"event_hook": "pre_call"})
    assert pre.event_hook_is_event_type("pre_call") is True
    assert pre.event_hook_is_event_type("post_call") is False
    both = build({"event_hook": ["pre_call", "post_call"]})
    assert both.event_hook_is_event_type("post_call") is True
    tagged = build({"event_hook": {"tags": {"health": "pre_call"}, "default": ["post_call"]}})
    assert tagged.event_hook_is_event_type("pre_call") is True
    assert tagged.event_hook_is_event_type("post_call") is True
    assert tagged.event_hook_is_event_type("during_call") is False
    assert build({"guardrail_name": "pii"}).event_hook_is_event_type("logging_only") is True


def test_native_select_gate() -> None:
    default_on = build({"guardrail_name": "pii", "default_on": True, "event_hook": "pre_call"})
    assert default_on.should_run_guardrail(EMPTY, "pre_call", "[]") is True
    assert default_on.should_run_guardrail(EMPTY, "post_call", "[]") is False
    assert default_on.should_run_guardrail(EMPTY, "pre_call", '["pii"]') is False
    disabled: dict[str, JsonValue] = {
        "metadata": {"user_api_key_metadata": {"disable_global_guardrails": True}},
    }
    assert default_on.should_run_guardrail(json.dumps(disabled), "pre_call", "[]") is False
    requested = build({"guardrail_name": "pii", "event_hook": "pre_call"})
    assert requested.should_run_guardrail(EMPTY, "pre_call", "[]") is False
    assert requested.should_run_guardrail(NAMED_GUARDRAIL, "pre_call", "[]") is True
    assert build({"guardrail_name": "pii"}).should_run_guardrail(EMPTY, "during_call", "[]") is True


def test_native_select_dynamic() -> None:
    guardrail = build({"guardrail_name": "pii"})
    data: dict[str, JsonValue] = {"guardrails": [{"pii": {"extra_body": {"threshold": 0.5}}}]}
    params = TypeAdapter(dict[str, JsonValue]).validate_json(
        guardrail.dynamic_request_body_params(json.dumps(data))
    )
    assert params == {"threshold": 0.5}
    empty = guardrail.dynamic_request_body_params(NAMED_GUARDRAIL)
    assert TypeAdapter(dict[str, JsonValue]).validate_json(empty) == {}


def test_native_select_errors() -> None:
    tagged = build({"guardrail_name": "pii", "event_hook": {"tags": {"health": "pre_call"}}})
    with pytest.raises(_native.NativeGuardrailError) as raised:
        tagged.should_run_guardrail(NAMED_GUARDRAIL, "pre_call", "[]")
    payload = TypeAdapter(ErrorPayload).validate_json(str(raised.value))
    assert payload["kind"] == "TagMode"
    assert payload["guardrail_name"] == "pii"
    requested = build({"guardrail_name": "pii", "event_hook": "pre_call"})
    with pytest.raises(_native.NativeGuardrailError) as raised:
        requested.should_run_guardrail('{"guardrails": "pii"}', "pre_call", "[]")
    payload = TypeAdapter(ErrorPayload).validate_json(str(raised.value))
    assert payload["kind"] == "RequestedNotList"


def test_native_records_sanitize() -> None:
    response: dict[str, JsonValue] = {
        "match": None,
        "score": SCORE,
        "secret_fields": {"authorization": "Bearer abcdefghijklmnop"},
        "outer": {"inner": LONG_VALUE, "gone": None},
    }
    cleaned = TypeAdapter(Sanitized).validate_json(
        _native.sanitize_guardrail_response(json.dumps(response))
    )
    assert cleaned.get("match") is None
    assert cleaned.get("secret_fields") is None
    assert cleaned.get("score") == SCORE
    outer = cleaned.get("outer")
    assert outer is not None
    assert outer.get("inner") == LONG_VALUE
    assert "gone" not in outer


def test_native_records_add() -> None:
    record_input: dict[str, JsonValue] = {
        "guardrail_response": {"verdict": "x", "score": SCORE},
        "guardrail_status": "success",
        "start_time": 1.0,
        "end_time": 2.0,
        "duration": 1.0,
        "event_type": "pre_call",
        "tracing_detail": {"guardrail_id": "g-1", "guardrail_cost": SCORE},
    }
    data_json, target_json = _native.add_guardrail_record(
        '{"model": "gpt-4o"}',
        json.dumps(record_input),
        json.dumps(record_context()),
    )
    record = first_record(data_json)
    assert record["guardrail_name"] == "pii"
    assert record["guardrail_status"] == "success"
    assert record["guardrail_mode"] == "pre_call"
    assert record["guardrail_response"] == {"verdict": "x", "score": SCORE}
    assert record["duration"] == 1.0
    assert record.get("guardrail_id") == "g-1"
    assert record.get("guardrail_cost") == SCORE
    target = TypeAdapter(LogTarget).validate_json(target_json)
    synced = TypeAdapter(list[TracedRecord]).validate_python(
        target["litellm_params"]["metadata"]["standard_logging_guardrail_information"]
    )
    assert synced == [record]


def test_native_records_summary() -> None:
    before = '{"messages": [{"role": "user", "content": "hi"}]}'
    after = '{"messages": [{"role": "user", "content": "**"}]}'
    masked = _native.summarize_guardrail_response(after, before, "pre_call")
    allowed = _native.summarize_guardrail_response(before, before, "pre_call")
    assert masked == '"mask"'
    assert allowed == '"allow"'
    assert _native.summarize_guardrail_response('{"verdict": "x"}', None, None) == '{"verdict":"x"}'
    assert _native.summarize_guardrail_response("null", None, None) == "{}"


def test_native_records_sync() -> None:
    entry: dict[str, JsonValue] = {"guardrail_name": "pii"}
    data: dict[str, JsonValue] = {"metadata": {"standard_logging_guardrail_information": [entry]}}
    target: dict[str, JsonValue] = {"litellm_params": {"metadata": {}}, "model_call_params": None}
    synced = TypeAdapter(LogTarget).validate_json(
        _native.sync_log_target(json.dumps(data), json.dumps(target))
    )
    copied = TypeAdapter(list[dict[str, JsonValue]]).validate_python(
        synced["litellm_params"]["metadata"]["standard_logging_guardrail_information"]
    )
    assert copied == [entry]
    assert synced["model_call_params"] is None


def test_native_records_messages() -> None:
    chat: dict[str, JsonValue] = {"messages": [{"role": "user", "content": "hi"}]}
    messages = _native.messages_for_call_type("completion", json.dumps(chat))
    assert messages is not None
    assert TypeAdapter(list[JsonValue]).validate_json(messages) == chat["messages"]
    responses = _native.messages_for_call_type("responses", '{"input": "hello"}')
    assert responses is not None
    assert TypeAdapter(list[JsonValue]).validate_json(responses) == [
        {"role": "user", "content": "hello"}
    ]
    assert _native.messages_for_call_type("embedding", json.dumps(chat)) is None
    assert _native.messages_for_call_type("completion", None) is None


def test_native_records_invoke() -> None:
    body: dict[str, JsonValue] = {"messages": [{"role": "user", "content": "hi"}]}
    text = json.dumps(body)
    invocation = _native.NativeGuardrailInvocation.begin("async_pre_call_hook", text, None)
    data_json, target_json = invocation.finish_success(text, text, json.dumps(record_context()))
    record = first_record(data_json)
    assert record["guardrail_status"] == "success"
    assert record["guardrail_response"] == "allow"
    assert record["guardrail_mode"] == "pre_call"
    assert record["start_time"] is not None
    target = TypeAdapter(LogTarget).validate_json(target_json)
    synced = TypeAdapter(list[TracedRecord]).validate_python(
        target["litellm_params"]["metadata"]["standard_logging_guardrail_information"]
    )
    assert synced == [record]
    with pytest.raises(_native.NativeGuardrailError) as raised:
        invocation.finish_success(text, text, json.dumps(record_context()))
    payload = TypeAdapter(ErrorPayload).validate_json(str(raised.value))
    assert payload["kind"] == "AlreadyFinished"


def test_native_records_own() -> None:
    body: dict[str, JsonValue] = {"messages": [{"role": "user", "content": "hi"}]}
    text = json.dumps(body)
    record_input: dict[str, JsonValue] = {
        "guardrail_response": {"verdict": "mask"},
        "guardrail_status": "guardrail_flagged",
    }
    invocation = _native.NativeGuardrailInvocation.begin("async_pre_call_hook", text, None)
    data_json, _ = invocation.record_own(
        text, json.dumps(record_input), json.dumps(record_context())
    )
    assert first_record(data_json)["guardrail_status"] == "guardrail_flagged"
    data_json, _ = invocation.finish_success(data_json, text, json.dumps(record_context()))
    assert first_record(data_json)["guardrail_status"] == "guardrail_flagged"


@pytest.mark.parametrize("hook", ["pre_call", "during_call", "post_call"])
def test_native_records_unknown(hook: str) -> None:
    invocation = _native.NativeGuardrailInvocation.begin("async_custom_hook", EMPTY, None)
    data_json, _ = invocation.finish_success(EMPTY, None, json.dumps(record_context(hook)))
    record = first_record(data_json)
    assert record["guardrail_mode"] == hook


def test_native_records_apply() -> None:
    inputs: dict[str, JsonValue] = {"texts": ["hi"]}
    invocation = _native.NativeGuardrailInvocation.begin(
        "apply_guardrail", EMPTY, json.dumps(inputs)
    )
    data_json, _ = invocation.finish_success(
        EMPTY, json.dumps(inputs), json.dumps(record_context("during_call"))
    )
    record = first_record(data_json)
    assert record["guardrail_response"] == "allow"
    assert record["guardrail_mode"] == "during_call"
