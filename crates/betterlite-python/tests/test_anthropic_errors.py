import pytest
from litellm.anthropic_interface import exceptions as litellm_exceptions

import betterlite.anthropic_interface.exceptions as betterlite_exceptions
from betterlite.anthropic_interface.exceptions import (
    ANTHROPIC_ERROR_TYPE_MAP,
    AnthropicErrorDetail,
    AnthropicErrorResponse,
    AnthropicErrorSseFrame,
    AnthropicErrorType,
    AnthropicExceptionMapping,
    anthropic_error_sse_frame,
)

STATUSES = [400, 401, 403, 404, 413, 429, 500, 529, 418, 200]
REQUEST_IDS: list[str | None] = [None, "", "req_a"]
CALL_IDS: list[str | None] = [None, "", "call_a"]
ERROR_MESSAGE = "boom"
LONG_INTEGER = "1" * 5000
DEEP_ARRAY = "[" * 100000
RAW_MESSAGES = [
    '{"detail": {"message": "bedrock"}}',
    '{"detail": {}}',
    '{"detail": {}, "Message": "aws"}',
    '{"detail": 5, "message": "generic"}',
    '{"detail": {"message": null}}',
    '{"detail": {"message": 7}}',
    '{"Message": "aws"}',
    '{"Message": "aws", "message": "generic"}',
    '{"Message": "", "message": "generic"}',
    '{"Message": "", "message": ""}',
    '{"Message": 5}',
    '{"Message": [1]}',
    '{"Message": {"key": "value"}}',
    '{"message": "generic"}',
    '{"message": false}',
    '{"message": {}}',
    "plain text",
    "[1, 2]",
    "null",
    '"quoted"',
    "7",
    "NaN",
    '{"type": "error", "error": {"type": "vendor", "message": "bad", "code": "x"}}',
    '{"type": "error", "error": {"type": "vendor", "message": "bad"}, "extra": 7}',
    '{"type": "error", "error": {"type": "vendor_error", "message": "bad"}, "request_id": null}',
    '{"type": "error", "error": {"type": "vendor_error", "message": "bad"}, "request_id": "old"}',
    '{"type": "error", "error": {"type": "vendor_error"}}',
    '{"type": "error", "error": {"message": "bad"}}',
    '{"type": "other", "error": {"type": "x", "message": "m"}}',
    '{"type": "error", "error": []}',
    '{"type": "error", "error": "text"}',
    '{"type": "error", "error": {"type": null, "message": "m"}}',
    '{"detail": {"message": "d"}, "Message": "aws", "message": "generic"}',
    pytest.param('{"message": ' + LONG_INTEGER + "}", id="long-integer"),
    pytest.param(DEEP_ARRAY, id="deep-array"),
    "{",
]


@pytest.mark.parametrize("status", STATUSES)
def test_error_type_matches(status: int) -> None:
    theirs = litellm_exceptions.AnthropicExceptionMapping.get_error_type(status)
    ours = AnthropicExceptionMapping.get_error_type(status)
    assert ours == theirs


@pytest.mark.parametrize("status", STATUSES)
@pytest.mark.parametrize("request_id", REQUEST_IDS)
def test_error_response_matches(status: int, request_id: str | None) -> None:
    their_call = litellm_exceptions.AnthropicExceptionMapping
    theirs: litellm_exceptions.AnthropicErrorResponse = their_call.create_error_response(
        status,
        ERROR_MESSAGE,
        request_id,
    )
    ours: AnthropicErrorResponse = AnthropicExceptionMapping.create_error_response(
        status,
        ERROR_MESSAGE,
        request_id,
    )
    assert ours == theirs
    if request_id:
        assert "request_id" in ours
    else:
        assert "request_id" not in ours


@pytest.mark.parametrize("raw_message", RAW_MESSAGES)
def test_extract_message_matches(raw_message: str) -> None:
    their_call = litellm_exceptions.AnthropicExceptionMapping
    theirs = their_call.extract_error_message(raw_message)
    ours = AnthropicExceptionMapping.extract_error_message(raw_message)
    assert ours == theirs


@pytest.mark.parametrize("raw_message", RAW_MESSAGES)
def test_transform_matches(raw_message: str) -> None:
    their_call = litellm_exceptions.AnthropicExceptionMapping
    theirs: litellm_exceptions.AnthropicErrorResponse = their_call.transform_to_anthropic_error(
        429,
        raw_message,
    )
    ours: AnthropicErrorResponse = AnthropicExceptionMapping.transform_to_anthropic_error(
        429,
        raw_message,
    )
    assert ours == theirs


@pytest.mark.parametrize("status", STATUSES)
def test_transform_status_matches(status: int) -> None:
    their_call = litellm_exceptions.AnthropicExceptionMapping
    theirs = their_call.transform_to_anthropic_error(status, ERROR_MESSAGE)
    ours = AnthropicExceptionMapping.transform_to_anthropic_error(status, ERROR_MESSAGE)
    assert ours == theirs


@pytest.mark.parametrize("request_id", REQUEST_IDS)
@pytest.mark.parametrize("raw_message", RAW_MESSAGES)
def test_transform_request_id(raw_message: str, request_id: str | None) -> None:
    their_call = litellm_exceptions.AnthropicExceptionMapping
    theirs = their_call.transform_to_anthropic_error(429, raw_message, request_id)
    ours = AnthropicExceptionMapping.transform_to_anthropic_error(429, raw_message, request_id)
    assert ours == theirs


@pytest.mark.parametrize("status", STATUSES)
@pytest.mark.parametrize("raw_message", RAW_MESSAGES)
def test_frame_matches(status: int, raw_message: str) -> None:
    theirs: litellm_exceptions.AnthropicErrorSseFrame = (
        litellm_exceptions.anthropic_error_sse_frame(status, raw_message)
    )
    ours: AnthropicErrorSseFrame = anthropic_error_sse_frame(status, raw_message)
    assert str(ours) == str(theirs)
    assert ours.status_code == theirs.status_code
    assert ours.error_response == theirs.error_response


@pytest.mark.parametrize("call_id", CALL_IDS)
def test_json_body_matches(call_id: str | None) -> None:
    theirs = litellm_exceptions.anthropic_error_sse_frame(429, "slow")
    ours = anthropic_error_sse_frame(429, "slow")
    assert ours.json_body(call_id) == theirs.json_body(call_id)
    assert ours.error_response == theirs.error_response


def test_json_body_none_keeps_body() -> None:
    frame = anthropic_error_sse_frame(429, "slow")
    body: AnthropicErrorResponse = frame.json_body(None)
    assert body is frame.error_response


def test_json_body_adds_call_id() -> None:
    frame = anthropic_error_sse_frame(429, "slow")
    detail: AnthropicErrorDetail = frame.json_body("call_a")["error"]
    assert detail.get("litellm_call_id") == "call_a"
    assert "litellm_call_id" not in frame.error_response["error"]


def test_exports_match() -> None:
    theirs: list[str] = litellm_exceptions.__all__
    ours: list[str] = betterlite_exceptions.__all__
    assert ours == theirs
    assert dict(ANTHROPIC_ERROR_TYPE_MAP) == dict(litellm_exceptions.ANTHROPIC_ERROR_TYPE_MAP)
    error_type: AnthropicErrorType = "api_error"
    assert ANTHROPIC_ERROR_TYPE_MAP[500] == error_type


def test_type_literal_matches() -> None:
    theirs = litellm_exceptions.AnthropicErrorType
    ours = AnthropicErrorType
    assert ours == theirs


def test_error_detail_keys_match() -> None:
    theirs = litellm_exceptions.AnthropicErrorDetail
    ours = AnthropicErrorDetail
    assert ours.__required_keys__ == theirs.__required_keys__
    assert ours.__optional_keys__ == theirs.__optional_keys__


def test_error_response_keys_match() -> None:
    theirs = litellm_exceptions.AnthropicErrorResponse
    ours = AnthropicErrorResponse
    assert ours.__required_keys__ == theirs.__required_keys__
    assert ours.__optional_keys__ == theirs.__optional_keys__
