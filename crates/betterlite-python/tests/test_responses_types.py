from collections.abc import Mapping, Sequence
from typing import Protocol

import litellm.types.llms.openai as litellm_openai  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1
import litellm.types.responses.main as litellm_responses  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/responses/main.py#L1
import pytest
from litellm.responses.main import mock_responses_api_response as litellm_mock_response  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L105
from pydantic import JsonValue, ValidationError

from betterlite.types.responses import (
    CustomToolCallOutputItem,
    DecodedResponseId,
    DeleteResponseResult,
    GenericResponseOutputItem,
    GenericResponseOutputItemContentAnnotation,
    OutputCodeInterpreterCall,
    OutputCodeInterpreterCallLog,
    OutputFunctionToolCall,
    OutputImageGenerationCall,
    OutputText,
    ResponseAPIUsage,
    ResponsesAPIResponse,
    ResponsesAPIStatus,
    ResponsesToolUsage,
    WebSearchToolUsage,
    build_code_interpreter_log_outputs,
    build_web_search_call,
    mock_responses_api_response,
)
from betterlite.types.utils.base import InitValue, ResponseObject

ANNOTATION_VALUES: dict[str, JsonValue] = {
    "type": "url_citation",
    "start_index": 0,
    "end_index": 4,
    "url": "https://example.com",
    "title": "Example",
}
TEXT_VALUES: dict[str, JsonValue] = {
    "type": "output_text",
    "text": "hello",
    "annotations": [ANNOTATION_VALUES],
}
FUNCTION_VALUES: dict[str, JsonValue] = {
    "arguments": "{}",
    "call_id": "call_1",
    "name": "lookup",
    "type": "function_call",
    "id": "fc_1",
    "status": "completed",
    "phase": "final_answer",
}
IMAGE_VALUES: dict[str, JsonValue] = {
    "type": "image_generation_call",
    "id": "img_1",
    "status": "completed",
    "result": "aGk=",
}
CODE_CALL_VALUES: dict[str, JsonValue] = {
    "type": "code_interpreter_call",
    "id": "ci_1",
    "code": "print(1)",
    "container_id": "ct_1",
    "status": "in_progress",
    "outputs": [{"type": "logs", "logs": "1\n"}],
}
CUSTOM_VALUES: dict[str, JsonValue] = {
    "type": "custom_tool_call",
    "call_id": "call_1",
    "name": "patch",
    "input": "*** Begin Patch",
}
GENERIC_VALUES: dict[str, JsonValue] = {
    "type": "message",
    "id": "msg_1",
    "status": "completed",
    "role": "assistant",
    "content": [TEXT_VALUES],
    "phase": "commentary",
}
DELETE_VALUES: dict[str, JsonValue] = {
    "id": "resp_1",
    "object": "response",
    "deleted": True,
}
USAGE_DETAIL_VALUES: dict[str, JsonValue] = {
    "input_tokens": 1,
    "input_tokens_details": {
        "cached_tokens": 2,
        "cached_tokens_details": {"text_tokens": 3, "unknown": 4},
        "unknown": 5,
    },
    "output_tokens": 6,
    "output_tokens_details": {"audio_tokens": 7, "unknown": 8},
    "total_tokens": 9,
}
TOOL_USAGE_VALUES: dict[str, JsonValue] = {
    "web_search": {"num_requests": 2, "extra": 9},
    "extra": 3,
}
USAGE_VALUES: dict[str, JsonValue] = {
    "input_tokens": 36,
    "input_tokens_details": {"cached_tokens": 0},
    "output_tokens": 87,
    "output_tokens_details": {},
    "total_tokens": 123,
}
RESPONSE_VALUES: dict[str, JsonValue] = {
    "id": "resp_1",
    "created_at": 1741476542,
    "error": None,
    "incomplete_details": {"reason": "max_output_tokens"},
    "instructions": "be brief",
    "metadata": {"trace": "t1"},
    "model": "gpt-4.1",
    "object": "response",
    "parallel_tool_calls": True,
    "temperature": 1.0,
    "tool_choice": "auto",
    "tools": [],
    "top_p": 1.0,
    "max_output_tokens": 16,
    "previous_response_id": "resp_0",
    "reasoning": {"effort": "low", "summary": None},
    "status": "completed",
    "text": {"format": {"type": "text"}},
    "usage": USAGE_VALUES,
    "user": "u1",
    "store": True,
}
REASONING_ITEM: dict[str, JsonValue] = {
    "type": "reasoning",
    "id": "rs_1",
    "status": None,
    "content": None,
    "encrypted_content": None,
    "summary": [],
    "kept": None,
}
OTHER_ITEM: dict[str, JsonValue] = {
    "type": "note",
    "id": "note_1",
    "status": None,
    "content": None,
    "encrypted_content": None,
}
MESSAGE_VALUES: dict[str, JsonValue] = {
    "type": "message", "id": "msg_4", "status": "completed", "role": "assistant", "content": [],
}
FUNCTION_TOOL: dict[str, JsonValue] = {
    "type": "function", "name": "f", "parameters": {"type": "object"},
}
WEB_SEARCH_TOOL: dict[str, JsonValue] = {"type": "web_search_preview"}
FUNCTION_TOOL_DEFAULTS: dict[str, JsonValue] = {
    "allowed_callers": None, "defer_loading": None, "description": None,
    "output_schema": None, "strict": None,
}
WEB_SEARCH_TOOL_DEFAULTS: dict[str, JsonValue] = {
    "search_content_types": None, "search_context_size": None, "user_location": None,
}
REFERENCE_TOOLS: list[JsonValue] = [
    {**FUNCTION_TOOL, **FUNCTION_TOOL_DEFAULTS},
    {**WEB_SEARCH_TOOL, **WEB_SEARCH_TOOL_DEFAULTS},
]
REASONING_RESPONSE: dict[str, JsonValue] = {
    "id": "resp_1", "created_at": 1, "output": [], "reasoning": {"effort": "low"},
}
WEB_SEARCH_ACTION: dict[str, JsonValue] = {
    "type": "search",
    "query": "cats",
    "queries": ["cats"],
    "sources": [
        {"type": "url", "url": "https://a.example"},
        {"type": "url", "url": ""},
    ],
}
WEB_SEARCH_CONTENT: dict[str, JsonValue] = {
    "content": [
        {"type": "web_search_result", "url": "https://a.example"},
        {"type": "web_search_result", "url": ""},
        {"type": "web_search_result"},
        {"type": "other", "url": "https://b.example"},
        "plain",
    ],
}
LOG_CONTENT_VALUES: tuple[JsonValue, ...] = (
    {"stdout": "out"}, {"stderr": "err"}, {}, {"stdout": "", "stderr": ""},
)
NON_LOG_VALUES: tuple[JsonValue, ...] = (None, "text", 5, [{"stdout": "out"}])
INPUT_TOKENS = 36
NEGATIVE_REQUESTS = -1
TOTAL_TOKENS = 123
CACHED_TEXT_TOKENS = 3
TOTAL_COST = 0.25


class Dumpable(Protocol):
    def model_dump(
        self, *, exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue: ...


class MockedResponse(Dumpable, Protocol):
    @property
    def output_text(self) -> str: ...

    def model_dump_json(self) -> str: ...


def reference_response(values: Mapping[str, InitValue]) -> MockedResponse:
    return litellm_openai.ResponsesAPIResponse.model_validate(values)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1388


class ReasoningValue(ResponseObject):
    effort: str


def assert_same(label: str, reference: Dumpable, ours: Dumpable) -> None:
    assert ours.model_dump() == reference.model_dump(), label
    assert ours.model_dump(exclude_unset=True) == reference.model_dump(exclude_unset=True), label
    assert ours.model_dump(exclude_none=True) == reference.model_dump(exclude_none=True), label


def as_mapping(value: JsonValue) -> dict[str, JsonValue]:
    match value:
        case dict():
            return value
        case bool() | int() | float() | str() | list() | None:
            raise AssertionError("model_dump returned a non-mapping")


def log_dumps(logs: Sequence[Dumpable] | None) -> list[JsonValue] | None:
    if logs is None:
        return None
    return [item.model_dump() for item in logs]


def assert_output_text(output: list[JsonValue], expected: str) -> None:
    ours = ResponsesAPIResponse(id="resp_1", created_at=1, output=output)
    reference = reference_response({"id": "resp_1", "created_at": 1, "output": output})
    assert ours.output_text == reference.output_text
    assert ours.output_text == expected


def hidden_params(response: ResponsesAPIResponse | DeleteResponseResult) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = response._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1424
    return params


def assert_hidden_fresh(
    first: ResponsesAPIResponse | DeleteResponseResult,
    second: ResponsesAPIResponse | DeleteResponseResult,
) -> None:
    assert hidden_params(first) == {}
    assert hidden_params(first) is not hidden_params(second)
    hidden_params(first)["kept"] = 1
    assert hidden_params(second) == {}
    assert "_hidden_params" not in first.model_dump()


def test_output_item_annotation() -> None:
    ours = GenericResponseOutputItemContentAnnotation(**ANNOTATION_VALUES)
    reference = litellm_responses.GenericResponseOutputItemContentAnnotation.model_validate(
        ANNOTATION_VALUES,
    )
    assert_same("annotation", reference, ours)
    assert ours.url == "https://example.com"
    assert ours["title"] == "Example"


def test_output_item_text() -> None:
    ours = OutputText(**TEXT_VALUES)
    reference = litellm_responses.OutputText.model_validate(TEXT_VALUES)
    assert_same("text", reference, ours)
    assert ours.annotations == [GenericResponseOutputItemContentAnnotation(**ANNOTATION_VALUES)]


def test_output_item_function_call() -> None:
    ours = OutputFunctionToolCall(**FUNCTION_VALUES)
    reference = litellm_responses.OutputFunctionToolCall.model_validate(FUNCTION_VALUES)
    assert_same("function_call", reference, ours)
    assert ours.phase == "final_answer"


def test_output_item_phase_default() -> None:
    values = {name: value for name, value in FUNCTION_VALUES.items() if name != "phase"}
    ours = OutputFunctionToolCall(**values)
    reference = litellm_responses.OutputFunctionToolCall.model_validate(values)
    assert_same("phase_default", reference, ours)
    assert ours.phase is None


def test_output_item_image_call() -> None:
    ours = OutputImageGenerationCall(**IMAGE_VALUES)
    reference = litellm_responses.OutputImageGenerationCall.model_validate(IMAGE_VALUES)
    assert_same("image_call", reference, ours)


def test_output_item_code_log() -> None:
    values: dict[str, JsonValue] = {"type": "logs", "logs": "out"}
    ours = OutputCodeInterpreterCallLog(**values)
    reference = litellm_responses.OutputCodeInterpreterCallLog.model_validate(values)
    assert_same("code_log", reference, ours)


def test_output_item_code_call() -> None:
    ours = OutputCodeInterpreterCall(**CODE_CALL_VALUES)
    reference = litellm_responses.OutputCodeInterpreterCall.model_validate(CODE_CALL_VALUES)
    assert_same("code_call", reference, ours)
    assert ours.outputs == [OutputCodeInterpreterCallLog(type="logs", logs="1\n")]


def test_output_item_custom_call() -> None:
    ours = CustomToolCallOutputItem(**CUSTOM_VALUES)
    reference = litellm_responses.CustomToolCallOutputItem.model_validate(CUSTOM_VALUES)
    assert_same("custom_call", reference, ours)
    assert ours.id is None
    assert ours.status is None


def test_output_item_custom_full() -> None:
    values = {**CUSTOM_VALUES, "id": "ct_1", "status": "in_progress"}
    ours = CustomToolCallOutputItem(**values)
    reference = litellm_responses.CustomToolCallOutputItem.model_validate(values)
    assert_same("custom_full", reference, ours)


def test_output_item_generic() -> None:
    ours = GenericResponseOutputItem(**GENERIC_VALUES)
    reference = litellm_responses.GenericResponseOutputItem.model_validate(GENERIC_VALUES)
    assert_same("generic", reference, ours)
    assert ours.content == [OutputText(**TEXT_VALUES)]


def test_output_item_required() -> None:
    with pytest.raises(ValidationError):
        OutputText(text="hi")
    missing_text: dict[str, str] = {"text": "hi"}
    with pytest.raises(ValidationError):
        litellm_responses.OutputText.model_validate(missing_text)
    without_status = {k: v for k, v in FUNCTION_VALUES.items() if k != "status"}
    with pytest.raises(ValidationError):
        OutputFunctionToolCall(**without_status)
    with pytest.raises(ValidationError):
        litellm_responses.OutputFunctionToolCall.model_validate(without_status)
    without_outputs = {k: v for k, v in CODE_CALL_VALUES.items() if k != "outputs"}
    with pytest.raises(ValidationError):
        OutputCodeInterpreterCall(**without_outputs)
    with pytest.raises(ValidationError):
        litellm_responses.OutputCodeInterpreterCall.model_validate(without_outputs)


def test_output_item_extras() -> None:
    values = {**TEXT_VALUES, "provider_field": "kept"}
    ours = OutputText(**values)
    reference = litellm_responses.OutputText.model_validate(values)
    assert_same("extras", reference, ours)
    assert ours["provider_field"] == "kept"


def test_builder_web_search() -> None:
    ours = build_web_search_call("abc", {"query": "cats"}, WEB_SEARCH_CONTENT)
    reference = litellm_responses.build_web_search_call(
        "abc", {"query": "cats"}, WEB_SEARCH_CONTENT,
    )
    assert_same("web_search", reference, ours)
    assert ours["action"] == WEB_SEARCH_ACTION


def test_builder_web_search_failed() -> None:
    content: dict[str, JsonValue] = {"type": "web_search_tool_result_error", "error_code": "x"}
    ours = build_web_search_call("abc", {}, {"content": content})
    reference = litellm_responses.build_web_search_call("abc", {}, {"content": content})
    assert_same("web_search_failed", reference, ours)
    assert ours["status"] == "failed"


def test_builder_web_search_status() -> None:
    ours = build_web_search_call("abc", {}, {}, status="searching")
    reference = litellm_responses.build_web_search_call("abc", {}, {}, status="searching")
    assert_same("web_search_status", reference, ours)
    assert ours["status"] == "searching"


def test_builder_web_search_input() -> None:
    ours = build_web_search_call("abc", "raw", "raw")
    reference = litellm_responses.build_web_search_call("abc", "raw", "raw")
    assert_same("web_search_input", reference, ours)
    assert ours["action"] == {"type": "search", "query": "", "queries": [], "sources": []}
    empty_query = build_web_search_call("abc", {"query": ""}, {})
    reference_empty = litellm_responses.build_web_search_call("abc", {"query": ""}, {})
    assert_same("web_search_empty", reference_empty, empty_query)


def test_builder_code_logs() -> None:
    content: dict[str, JsonValue] = {"stdout": "out", "stderr": "err"}
    ours = build_code_interpreter_log_outputs(content)
    reference = litellm_responses.build_code_interpreter_log_outputs(content)
    assert log_dumps(ours) == log_dumps(reference)
    assert log_dumps(ours) == [{"type": "logs", "logs": "outSTDERR: err"}]


def test_builder_code_logs_text() -> None:
    for content in LOG_CONTENT_VALUES:
        ours = build_code_interpreter_log_outputs(content)
        reference = litellm_responses.build_code_interpreter_log_outputs(content)
        assert log_dumps(ours) == log_dumps(reference), str(content)


def test_builder_code_logs_none() -> None:
    for content in NON_LOG_VALUES:
        assert build_code_interpreter_log_outputs(content) is None
        assert litellm_responses.build_code_interpreter_log_outputs(content) is None


def test_delete_result_dump() -> None:
    ours = DeleteResponseResult(**DELETE_VALUES)
    reference = litellm_responses.DeleteResponseResult.model_validate(DELETE_VALUES)
    assert_same("delete_result", reference, ours)
    assert ours.deleted is True


def test_delete_result_hidden() -> None:
    first = DeleteResponseResult(**DELETE_VALUES)
    assert_hidden_fresh(first, DeleteResponseResult(**DELETE_VALUES))


def test_delete_result_required() -> None:
    missing: dict[str, JsonValue] = {}
    with pytest.raises(ValidationError):
        DeleteResponseResult(**missing)
    with pytest.raises(ValidationError):
        litellm_responses.DeleteResponseResult.model_validate(missing)


def test_api_response_dump() -> None:
    item = GenericResponseOutputItem(**GENERIC_VALUES)
    ours = ResponsesAPIResponse(**RESPONSE_VALUES, output=[item])
    reference = reference_response(
        {
            **RESPONSE_VALUES,
            "output": [litellm_responses.GenericResponseOutputItem.model_validate(GENERIC_VALUES)],
        },
    )
    assert_same("api_response", reference, ours)
    assert ours.model == "gpt-4.1"
    assert ours.error is None


def test_api_response_usage_dict() -> None:
    ours = ResponsesAPIResponse(**RESPONSE_VALUES, output=[])
    reference = reference_response({**RESPONSE_VALUES, "output": []})
    assert_same("usage_dict", reference, ours)
    assert ours.usage is not None
    assert ours.usage.total_tokens == TOTAL_TOKENS
    assert ours.usage.input_tokens_details is not None
    assert ours.usage.input_tokens_details.cached_tokens == 0
    assert ours.usage.cost is None


def test_api_response_tools_dict() -> None:
    tools = [FUNCTION_TOOL, WEB_SEARCH_TOOL]
    values = {**RESPONSE_VALUES, "output": [], "tools": tools}
    assert as_mapping(ResponsesAPIResponse(**values).model_dump())["tools"] == tools
    reference_dump: JsonValue = reference_response(values).model_dump()
    assert as_mapping(reference_dump)["tools"] == REFERENCE_TOOLS


def test_api_response_reasoning() -> None:
    ours = ResponsesAPIResponse(
        id="resp_1", created_at=1, output=[], reasoning=ReasoningValue(effort="low"),
    )
    assert_same("reasoning", reference_response(REASONING_RESPONSE), ours)
    assert ours.reasoning == {"effort": "low"}


def test_api_response_nulls() -> None:
    output = [dict(REASONING_ITEM), dict(OTHER_ITEM)]
    ours = ResponsesAPIResponse(id="resp_1", created_at=1, output=output)
    reference = reference_response({"id": "resp_1", "created_at": 1, "output": output})
    reference_dump: JsonValue = reference.model_dump()
    reference_unset: JsonValue = reference.model_dump(exclude_unset=True)
    assert ours.model_dump() == reference_dump
    assert ours.model_dump(exclude_unset=True) == reference_unset
    dumped = as_mapping(ours.model_dump())
    assert dumped["output"] == [
        {"type": "reasoning", "id": "rs_1", "summary": [], "kept": None},
        OTHER_ITEM,
    ]
    assert ours.output == output


def test_api_response_dump_json() -> None:
    output = [dict(REASONING_ITEM)]
    ours = ResponsesAPIResponse(id="resp_1", created_at=1, output=output)
    reference = reference_response({"id": "resp_1", "created_at": 1, "output": output})
    assert '"encrypted_content":null' in ours.model_dump_json()
    assert '"encrypted_content":null' not in reference.model_dump_json()


def test_api_response_output_text() -> None:
    output: list[JsonValue] = [
        {"type": "message", "id": "msg_1", "status": "completed", "role": "assistant",
         "content": [{"type": "output_text", "text": "hello"},
                     {"type": "output_text", "text": None}]},
        {"type": "message", "id": "msg_2", "status": "completed", "role": "assistant",
         "content": [{"type": "refusal", "refusal": "no"}]},
        {"type": "function_call", "id": "fc_1", "content": []},
    ]
    assert_output_text(output, "hello")


def test_output_text_objects() -> None:
    ours = ResponsesAPIResponse(
        id="resp_1",
        created_at=1,
        output=[
            GenericResponseOutputItem(
                type="message", id="msg_1", status="completed", role="assistant",
                content=[OutputText(type="output_text", text="hello", annotations=[])],
            ),
            OutputFunctionToolCall(**FUNCTION_VALUES),
        ],
    )
    reference = reference_response(
        {
            "id": "resp_1",
            "created_at": 1,
            "output": [
                litellm_responses.GenericResponseOutputItem(
                    type="message", id="msg_1", status="completed", role="assistant",
                    content=[litellm_responses.OutputText(
                        type="output_text", text="hello", annotations=[],
                    )],
                ),
                litellm_responses.OutputFunctionToolCall.model_validate(FUNCTION_VALUES),
            ],
        },
    )
    assert ours.output_text == reference.output_text
    assert ours.output_text == "hello"
    assert_same("objects", reference, ours)


def test_output_text_empty() -> None:
    output: list[JsonValue] = [
        {"type": "message", "id": "msg_4", "status": "completed", "role": "assistant",
         "content": []},
    ]
    assert_output_text(output, "")


def test_api_response_hidden() -> None:
    first = ResponsesAPIResponse(id="resp_1", created_at=1, output=[])
    assert_hidden_fresh(first, ResponsesAPIResponse(id="resp_2", created_at=1, output=[]))


def test_api_response_contracts() -> None:
    assert ResponsesAPIStatus == litellm_openai.ResponsesAPIStatus
    reference = litellm_responses.DecodedResponseId
    assert DecodedResponseId.__required_keys__ == reference.__required_keys__
    assert DecodedResponseId.__optional_keys__ == reference.__optional_keys__
    assert sorted(DecodedResponseId.__annotations__) == sorted(reference.__annotations__)


def test_usage_dump() -> None:
    ours = ResponseAPIUsage(**USAGE_VALUES)
    reference = litellm_openai.ResponseAPIUsage.model_validate(USAGE_VALUES)
    assert_same("usage", reference, ours)
    assert ours.input_tokens == INPUT_TOKENS
    assert ours.total_tokens == TOTAL_TOKENS


def test_usage_details() -> None:
    ours = ResponseAPIUsage(**USAGE_DETAIL_VALUES)
    reference = litellm_openai.ResponseAPIUsage.model_validate(USAGE_DETAIL_VALUES)
    assert_same("details", reference, ours)
    assert ours.input_tokens_details is not None
    assert ours.input_tokens_details.cached_tokens_details is not None
    assert ours.input_tokens_details.cached_tokens_details.text_tokens == CACHED_TEXT_TOKENS


def test_usage_cost() -> None:
    dict_cost: dict[str, JsonValue] = {
        "input_tokens": 1, "output_tokens": 2, "total_tokens": 3,
        "cost": {"total_cost": 0.25, "other": 1},
    }
    missing_cost: dict[str, JsonValue] = {
        "input_tokens": 1, "output_tokens": 2, "total_tokens": 3, "cost": {"other": 1},
    }
    float_cost: dict[str, JsonValue] = {
        "input_tokens": 1, "output_tokens": 2, "total_tokens": 3, "cost": 0.5,
    }
    ours = ResponseAPIUsage(**dict_cost)
    reference = litellm_openai.ResponseAPIUsage.model_validate(dict_cost)
    assert_same("cost_dict", reference, ours)
    assert ours.cost == TOTAL_COST
    ours_missing = ResponseAPIUsage(**missing_cost)
    reference_missing = litellm_openai.ResponseAPIUsage.model_validate(missing_cost)
    assert_same("cost_missing", reference_missing, ours_missing)
    assert ours_missing.cost is None
    ours_float = ResponseAPIUsage(**float_cost)
    reference_float = litellm_openai.ResponseAPIUsage.model_validate(float_cost)
    assert_same("cost_float", reference_float, ours_float)


def test_usage_classes() -> None:
    ours = ResponsesToolUsage(**TOOL_USAGE_VALUES)
    reference = litellm_openai.ResponsesToolUsage.model_validate(TOOL_USAGE_VALUES)
    assert_same("tool_usage", reference, ours)
    assert ours.web_search == WebSearchToolUsage(num_requests=2)
    empty = ResponsesToolUsage()
    reference_empty = litellm_openai.ResponsesToolUsage()
    assert_same("tool_usage_empty", reference_empty, empty)
    with pytest.raises(ValidationError):
        WebSearchToolUsage(num_requests=NEGATIVE_REQUESTS)
    with pytest.raises(ValidationError):
        litellm_openai.WebSearchToolUsage(num_requests=NEGATIVE_REQUESTS)
    with pytest.raises(ValidationError):
        ResponsesToolUsage(web_search={"num_requests": NEGATIVE_REQUESTS})
    negative_usage: dict[str, dict[str, int]] = {"web_search": {"num_requests": NEGATIVE_REQUESTS}}
    with pytest.raises(ValidationError):
        litellm_openai.ResponsesToolUsage.model_validate(negative_usage)


def test_usage_frozen() -> None:
    tool = WebSearchToolUsage(num_requests=1)
    with pytest.raises(ValidationError, match="frozen"):
        tool["num_requests"] = 2
    usage = ResponsesToolUsage(web_search=tool)
    with pytest.raises(ValidationError, match="frozen"):
        usage["web_search"] = None


def test_mock_default() -> None:
    ours = mock_responses_api_response()
    reference: MockedResponse = litellm_mock_response()
    assert ours.model_dump() == reference.model_dump()
    assert ours.output_text == reference.output_text
    assert ours.output_text.startswith("In a peaceful grove")


def test_mock_custom() -> None:
    ours = mock_responses_api_response("custom text")
    reference: MockedResponse = litellm_mock_response("custom text")
    assert ours.model_dump() == reference.model_dump()
    assert ours.output_text == "custom text"
