import copy
import importlib
import json
import logging
import threading
import time
from datetime import UTC, datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import ModuleType, TracebackType
from typing import ClassVar, Final, Protocol, Self

import httpx2
import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

import betterlite
import betterlite._core_helpers as helpers_package
import betterlite.litellm_core_utils.core_helpers as reexported_helpers
from betterlite import utils as utils_module
from betterlite._core_helpers import (
    drop_params_env_flag,
    drop_params_flag,
    filter_exceptions_from_params,
    filter_internal_params,
    get_litellm_metadata_from_kwargs,
    get_metadata_variable_name_from_kwargs,
    get_or_create_metadata_bucket,
    get_provider_response_headers_from_hidden_params,
    independent_snapshot,
    is_codex_user_agent,
    map_finish_reason,
    normalize_drop_params,
    preserve_upstream_non_openai_attributes,
    process_response_headers,
    reconstruct_model_name,
    remove_index_from_tool_calls,
    remove_items_at_indices,
    safe_deep_copy,
    safe_divide,
    safe_divide_seconds,
    set_provider_response_headers_in_hidden_params,
    set_response_cost_in_hidden_params,
)
from betterlite._core_helpers.values import PythonValue
from betterlite._cost import completion_cost, cost_per_token, response_cost_calculator
from betterlite._cost.objects import private_fields
from betterlite._model_info import (
    get_max_tokens,
    get_model_cost_mutation_generation,
    get_model_info,
    get_provider_info,
)
from betterlite._provider import get_llm_provider
from betterlite._tokens import (
    create_pretrained_tokenizer,
    create_tokenizer,
    decode,
    encode,
    token_counter,
)
from betterlite.budget_manager import (
    DAYS_IN_A_MONTH,
    DAYS_IN_A_WEEK,
    DAYS_IN_A_YEAR,
    HOURS_IN_A_DAY,
    BudgetManager,
)
from betterlite.types.utils.choices import map_finish_reason as parts_map_finish_reason
from betterlite.types.utils.response import ModelResponse, ModelResponseStream

JSON_TYPE: Final = "application/json"
COST_FILE: Final = "user_cost.json"
PROJECT: Final = "proj"
CREATED_AT: Final = 1000.0
METADATA_KEY: Final = "metadata"
LITELLM_METADATA_KEY: Final = "litellm_metadata"
PARENT_SPAN: Final = "litellm_parent_otel_span"
MARKER_KEY: Final = "k"
MARKER_VALUE: Final = "marker-value"
PLACEHOLDER: Final = "placeholder"
CUSTOM_ATTR: Final = "custom_attr"
CUSTOM_VALUE: Final = "v"
BOOM: Final = "boom"
FLAG_NAME: Final = "TEST_FLAG"
CHAT_MODEL: Final = "gpt-4o"
BUDGET: Final = 10.0
ZERO_COST: Final = 0.0
COST: Final = 0.25
ALICE: Final = "alice"
BOB: Final = "bob"
CAROL: Final = "carol"
DAVE: Final = "dave"
ZOE: Final = "zoe"
NOT_FOUND_MESSAGE: Final = "User Dictionary not found!"
LOCAL_DICT_MESSAGE: Final = "user dict from local: {}"
HOSTED: Final = "hosted"
UNKNOWN_CLIENT: Final = "remote"
GET_BUDGET: Final = "/get_budget"
SET_BUDGET: Final = "/set_budget"
HOST: Final = "127.0.0.1"
OK_STATUS: Final = 200
LOAD_REQUESTS: Final = 2
SAVE_REQUESTS: Final = 2
SAFE_MEMORY_ON: Final = True

HELPER_NAMES: Final[frozenset[str]] = frozenset(
    {
        "drop_params_env_flag", "drop_params_flag", "filter_exceptions_from_params",
        "filter_internal_params", "get_litellm_metadata_from_kwargs",
        "get_metadata_variable_name_from_kwargs", "get_or_create_metadata_bucket",
        "get_provider_response_headers_from_hidden_params", "independent_snapshot",
        "is_codex_user_agent", "map_finish_reason", "normalize_drop_params",
        "preserve_upstream_non_openai_attributes", "process_response_headers",
        "reconstruct_model_name", "remove_index_from_tool_calls", "remove_items_at_indices",
        "safe_deep_copy", "safe_divide", "safe_divide_seconds",
        "set_provider_response_headers_in_hidden_params", "set_response_cost_in_hidden_params",
    }
)
UTILS_NAMES: Final[frozenset[str]] = frozenset(
    {
        "completion_cost", "cost_per_token", "create_pretrained_tokenizer", "create_tokenizer",
        "decode", "encode", "get_llm_provider", "get_max_tokens",
        "get_model_cost_mutation_generation", "get_model_info", "get_provider_info",
        "response_cost_calculator", "token_counter",
    }
)
NORMALIZE_VALUES: Final[tuple[PythonValue, ...]] = (
    None, True, "true", " TRUE ", "on", "1", 1, 0, "banana", 2, {}, "false", "off",
)
ENVIRONMENTS: Final[tuple[dict[str, str], ...]] = (
    {},
    {"LITELLM_DROP_PARAMS": "true"},
    {"LITELLM_DROP_PARAMS": "  "},
    {"LITELLM_DROP_PARAMS": "banana"},
    {"LITELLM_DROP_PARAMS": "0"},
    {"LITELLM_DROP_PARAMS": "off"},
    {"LITELLM_DROP_PARAMS": "FALSE"},
)
CODEX_AGENTS: Final[tuple[str, ...]] = (
    "codex-cli/1.0", "Codex_v", "codex /x", "xcodex", "openai",
)
FINISH_REASONS: Final[tuple[str, ...]] = (
    "stop", "length", "tool_calls", "content_filtered", "error", "weird",
)
HEADER_CASES: Final[tuple[dict[str, str], ...]] = (
    {"content-type": "text/plain", "llm_provider-x": "y", "x-litellm-z": "w", "b": "c"},
    {"x-ratelimit-remaining-requests": "9"},
    {},
)
PROJECTED_MESSAGES: Final[list[dict[str, JsonValue]]] = [
    {"role": "user", "content": "ab"},
    {"role": "user", "content": "c"},
]
USAGE_DATA: Final[dict[str, JsonValue]] = {
    "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2,
}
HOSTED_USERS: Final[dict[str, dict[str, JsonValue]]] = {ALICE: {"total_budget": BUDGET}}
HOSTED_REPLY: Final[dict[str, JsonValue]] = {"status": "ok", "data": HOSTED_USERS}
ERROR_REPLY: Final[dict[str, JsonValue]] = {"status": "error"}
SAVED_REPLY: Final[dict[str, JsonValue]] = {"status": "success"}
HOSTED_SAVE_BODY: Final[dict[str, JsonValue]] = {
    "project_name": PROJECT, "user_dict": HOSTED_USERS,
}
JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ReferenceResponse(Protocol):
    __pydantic_private__: dict[str, PythonValue] | None

    def model_dump(self, **kwargs: PythonValue) -> PythonValue: ...


class ReferenceCall(Protocol):
    def __call__(
        self,
        *args: PythonValue | ReferenceResponse | logging.Logger | httpx2.Headers,
        **kwargs: PythonValue | ReferenceResponse | logging.Logger | httpx2.Headers,
    ) -> PythonValue: ...


class HelpersModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class ResponseFactory(Protocol):
    def __call__(
        self, **kwargs: PythonValue | ReferenceResponse | logging.Logger | httpx2.Headers
    ) -> ReferenceResponse: ...


class ResponseModule(Protocol):
    def __getattr__(self, name: str) -> ResponseFactory: ...


class BudgetShape(Protocol):
    user_dict: dict[str, dict[str, JsonValue]]

    def create_budget(
        self, total_budget: float, user: str, duration: str | None = ..., created_at: float = ...
    ) -> dict[str, JsonValue]: ...
    def projected_cost(
        self, model: str, messages: list[dict[str, JsonValue]], user: str
    ) -> float: ...
    def update_cost(
        self,
        user: str,
        completion_obj: ReferenceResponse | None = ...,
        model: str | None = ...,
        input_text: str | None = ...,
        output_text: str | None = ...,
    ) -> dict[str, JsonValue]: ...
    def get_total_budget(self, user: str) -> JsonValue: ...
    def get_current_cost(self, user: str) -> JsonValue: ...
    def get_model_cost(self, user: str) -> JsonValue: ...
    def is_valid_user(self, user: str) -> bool: ...
    def get_users(self) -> list[str]: ...
    def reset_cost(self, user: str) -> dict[str, JsonValue]: ...
    def reset_on_duration(self, user: str) -> None: ...
    def update_budget_all_users(self) -> None: ...
    def save_data(self) -> JsonValue: ...


class BudgetFactory(Protocol):
    def __call__(
        self,
        project_name: str,
        client_type: str = ...,
        api_base: str | None = ...,
        headers: dict[str, str] | None = ...,
    ) -> BudgetShape: ...


class BudgetModule(Protocol):
    def __getattr__(self, name: str) -> BudgetFactory: ...


LITELLM: ModuleType = importlib.import_module("litellm")
BETTERLITE: ModuleType = betterlite
LITELLM_HELPERS: HelpersModule = importlib.import_module(
    "litellm.litellm_core_utils.core_helpers"
)
LITELLM_RESPONSES: ResponseModule = importlib.import_module("litellm")
LITELLM_BUDGET: BudgetModule = importlib.import_module("litellm.budget_manager")


class Router(ModelResponse):
    ...


class Logging(ModelResponse):
    ...


def helper_function() -> None:
    ...


class OneSecondClock:
    now: float

    def __init__(self, now: float) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


def fake_token_counter(
    model: str = "", text: str | None = None, messages: list[dict[str, JsonValue]] | None = None
) -> int:
    if text is not None:
        return len(model) + len(text)
    if messages is not None:
        return len(model) + len(messages)
    return len(model)


def fake_cost_per_token(
    *, model: str, prompt_tokens: int, completion_tokens: int
) -> tuple[float, float]:
    prompt_cost = len(model) / 10 + prompt_tokens / 100
    return (prompt_cost, completion_tokens / 100)


def fake_completion_cost(*, completion_response: PythonValue) -> float:
    if completion_response is None:
        return 0.0
    return 0.5


def patch_costs(monkeypatch: pytest.MonkeyPatch) -> None:
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "token_counter", fake_token_counter)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "token_counter", fake_token_counter)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "cost_per_token", fake_cost_per_token)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "cost_per_token", fake_cost_per_token)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "completion_cost", fake_completion_cost)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "completion_cost", fake_completion_cost)


def fields_of(value: PythonValue) -> dict[str, PythonValue]:
    match value:
        case dict() as fields:
            return fields
    raise TypeError("expected an object")


def error_text(value: PythonValue) -> str:
    match value:
        case BaseException() as error:
            return str(error)
    raise TypeError("expected an exception")


def hidden_of(response: ReferenceResponse) -> dict[str, PythonValue]:
    private = response.__pydantic_private__
    if private is None:
        return {}
    return fields_of(private["_hidden_params"])


def ours_hidden(response: ModelResponse | ModelResponseStream) -> dict[str, JsonValue]:
    value = private_fields(response)["_hidden_params"]
    match value:
        case dict() as fields:
            return fields
    raise TypeError("expected hidden params")


def copy_payload(span: datetime) -> dict[str, PythonValue]:
    metadata: dict[str, PythonValue] = {PARENT_SPAN: span, MARKER_KEY: MARKER_VALUE}
    other: dict[str, PythonValue] = {PARENT_SPAN: span}
    return {METADATA_KEY: metadata, LITELLM_METADATA_KEY: other}


def failing_payload(span: datetime) -> dict[str, PythonValue]:
    generator = (item for item in [1])
    metadata: dict[str, PythonValue] = {PARENT_SPAN: span, "g": generator}
    return {METADATA_KEY: metadata}


def cyclic_payload() -> dict[str, PythonValue]:
    payload: dict[str, PythonValue] = {}
    payload["self"] = payload
    return payload


def filter_payload() -> dict[str, PythonValue]:
    return {
        "class": ValueError,
        "function": helper_function,
        "error": ValueError(BOOM),
        "none": None,
        "router": Router(),
        "logging": Logging(),
        "list": [ValueError(BOOM), {"router": Router()}, 1],
    }


def join_saves(marker: set[threading.Thread]) -> None:
    for thread in set(threading.enumerate()) - marker:
        if not thread.daemon:
            thread.join(timeout=10)


class RecordingServer(ThreadingHTTPServer):
    replies: dict[str, JsonValue]
    requests: list[dict[str, JsonValue]]
    thread: threading.Thread

    def __init__(self) -> None:
        super().__init__((HOST, 0), RecordingHandler)
        self.replies = {}
        self.requests = []
        RecordingHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    @override
    def __enter__(self) -> Self:
        return self

    @override
    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class RecordingHandler(BaseHTTPRequestHandler):
    owner: ClassVar[RecordingServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        parsed: JsonValue = None if not body else JSON_ADAPTER.validate_json(body)
        recorded: dict[str, JsonValue] = {
            "path": self.path,
            "content_type": self.headers.get("Content-Type"),
            "body": parsed,
        }
        self.owner.requests.append(recorded)
        reply = self.owner.replies[self.path]
        payload = json.dumps(reply).encode()
        self.send_response(OK_STATUS)
        self.send_header("content-type", JSON_TYPE)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def test_helper_codex_agent() -> None:
    for agent in CODEX_AGENTS:
        expected = LITELLM_HELPERS.is_codex_user_agent(agent)
        assert is_codex_user_agent(agent) == expected


def test_helper_safe_divide() -> None:
    cases: tuple[tuple[float, float, float], ...] = (
        (4.0, 2.0, 0.0),
        (4.0, 0.0, 0.0),
        (4.0, 0.0, 7.0),
        (4.0, -2.0, 0.0),
    )
    for numerator, denominator, default in cases:
        expected = LITELLM_HELPERS.safe_divide(numerator, denominator, default)
        assert safe_divide(numerator, denominator, default) == expected


def test_helper_divide_seconds() -> None:
    cases: tuple[tuple[float, float, float | None], ...] = (
        (10.0, 0.0, None),
        (10.0, 0.0, 5.0),
        (10.0, -1.0, None),
        (10.0, 4.0, None),
    )
    for seconds, denominator, default in cases:
        expected = LITELLM_HELPERS.safe_divide_seconds(seconds, denominator, default)
        assert safe_divide_seconds(seconds, denominator, default) == expected


def test_helper_normalize_flag() -> None:
    for value in NORMALIZE_VALUES:
        expected = LITELLM_HELPERS.normalize_drop_params(value)
        assert normalize_drop_params(value) == expected


def test_helper_drop_flag() -> None:
    logger = logging.getLogger("b166-drop")
    for value in NORMALIZE_VALUES:
        expected = LITELLM_HELPERS.drop_params_flag(value, FLAG_NAME, logger)
        assert drop_params_flag(value, FLAG_NAME, logger) == expected


def test_helper_drop_env() -> None:
    logger = logging.getLogger("b166-env")
    for environ in ENVIRONMENTS:
        expected = LITELLM_HELPERS.drop_params_env_flag(environ, logger)
        assert drop_params_env_flag(environ, logger) == expected


def test_helper_finish_reason() -> None:
    for reason in FINISH_REASONS:
        expected = LITELLM_HELPERS.map_finish_reason(reason)
        assert map_finish_reason(reason) == expected


def test_helper_tool_call_index() -> None:
    tool_calls: list[JsonValue] = [{"index": 0, "id": "a"}, "s", {"id": "b"}]
    reference_messages: list[dict[str, JsonValue]] = [
        {"role": "user", "content": "x", "tool_calls": tool_calls},
        {"role": "user", "content": "y", "tool_calls": {"index": 1}},
    ]
    ours_messages: list[dict[str, JsonValue]] = copy.deepcopy(reference_messages)
    LITELLM_HELPERS.remove_index_from_tool_calls(reference_messages)
    remove_index_from_tool_calls(ours_messages)
    assert ours_messages == reference_messages
    expected_calls: list[JsonValue] = [{"id": "a"}, "s", {"id": "b"}]
    assert ours_messages[0]["tool_calls"] == expected_calls
    remove_index_from_tool_calls(None)
    LITELLM_HELPERS.remove_index_from_tool_calls(None)


def test_helper_remove_items() -> None:
    items: list[JsonValue] = ["a", "b", "c", "d"]
    reference_items: list[JsonValue] = list(items)
    ours_items: list[JsonValue] = list(items)
    LITELLM_HELPERS.remove_items_at_indices(reference_items, [0, 1, 9, 1, -1])
    remove_items_at_indices(ours_items, [0, 1, 9, 1, -1])
    expected: list[JsonValue] = ["c", "d"]
    assert ours_items == reference_items == expected
    no_items: list[JsonValue] | None = None
    remove_items_at_indices(no_items, [0])
    LITELLM_HELPERS.remove_items_at_indices(no_items, [0])


def test_helper_metadata_name() -> None:
    cases: tuple[dict[str, PythonValue], ...] = (
        {},
        {METADATA_KEY: {}},
        {LITELLM_METADATA_KEY: None},
        {LITELLM_METADATA_KEY: {}},
    )
    for kwargs in cases:
        expected = LITELLM_HELPERS.get_metadata_variable_name_from_kwargs(kwargs)
        assert get_metadata_variable_name_from_kwargs(kwargs) == expected


def test_helper_metadata_bucket() -> None:
    ours_request: dict[str, PythonValue] = {}
    reference_request: dict[str, PythonValue] = {}
    expected = LITELLM_HELPERS.get_or_create_metadata_bucket(reference_request)
    assert get_or_create_metadata_bucket(ours_request) == expected
    assert ours_request == reference_request == {METADATA_KEY: {}}
    existing: dict[str, PythonValue] = {METADATA_KEY: {}}
    key, bucket = get_or_create_metadata_bucket(existing)
    assert key == METADATA_KEY
    assert bucket is existing[METADATA_KEY]


def test_helper_meta_kwargs() -> None:
    cases: tuple[dict[str, PythonValue], ...] = (
        {},
        {"litellm_params": None},
        {"litellm_params": {}},
        {"litellm_params": {METADATA_KEY: {}}},
        {"litellm_params": {LITELLM_METADATA_KEY: {}}},
        {"litellm_params": {METADATA_KEY: None, LITELLM_METADATA_KEY: {"m": 2}}},
        {
            "litellm_params": {
                METADATA_KEY: {"user_api_key": "k"},
                LITELLM_METADATA_KEY: {"m": 2},
            }
        },
        {"litellm_params": {METADATA_KEY: {"user_api_key": "k"}}},
    )
    for kwargs in cases:
        expected = LITELLM_HELPERS.get_litellm_metadata_from_kwargs(copy.deepcopy(kwargs))
        assert get_litellm_metadata_from_kwargs(copy.deepcopy(kwargs)) == expected


def test_helper_reconstruct_name() -> None:
    cases: tuple[tuple[str, str | None, dict[str, PythonValue]], ...] = (
        ("claude", "bedrock", {}),
        ("claude", "anthropic", {}),
        ("bedrock/x", "bedrock", {"deployment": "anthropic/y"}),
        ("x", "bedrock", {"deployment": "claude"}),
        ("", "bedrock", {}),
        ("x", None, {}),
    )
    for model_name, provider, metadata in cases:
        expected = LITELLM_HELPERS.reconstruct_model_name(model_name, provider, metadata)
        assert reconstruct_model_name(model_name, provider, metadata) == expected


def test_helper_response_headers() -> None:
    for headers in HEADER_CASES:
        expected = LITELLM_HELPERS.process_response_headers(headers)
        assert process_response_headers(headers) == expected
        preserved = LITELLM_HELPERS.process_response_headers(
            headers, preserve_litellm_internal_headers=True
        )
        assert process_response_headers(
            headers, preserve_litellm_internal_headers=True
        ) == preserved
    raw_headers = httpx2.Headers({"content-type": "text/plain", "x-litellm-z": "w"})
    raw_expected = LITELLM_HELPERS.process_response_headers(
        raw_headers, preserve_litellm_internal_headers=True
    )
    assert process_response_headers(
        raw_headers, preserve_litellm_internal_headers=True
    ) == raw_expected


def test_helper_hidden_cost() -> None:
    reference_response = LITELLM_RESPONSES.ModelResponse()
    ours_response = ModelResponse()
    set_response_cost_in_hidden_params(ours_response, COST)
    LITELLM_HELPERS.set_response_cost_in_hidden_params(reference_response, COST)
    assert ours_hidden(ours_response) == hidden_of(reference_response)
    headers: dict[str, str] = {"A": "b", "llm_provider-x": "y", "x-litellm-z": "w"}
    set_provider_response_headers_in_hidden_params(ours_response, headers)
    LITELLM_HELPERS.set_provider_response_headers_in_hidden_params(reference_response, headers)
    assert ours_hidden(ours_response) == hidden_of(reference_response)
    expected = LITELLM_HELPERS.get_provider_response_headers_from_hidden_params(
        reference_response
    )
    assert get_provider_response_headers_from_hidden_params(ours_response) == expected
    fresh = ModelResponse()
    set_response_cost_in_hidden_params(fresh, None)
    assert "additional_headers" not in ours_hidden(fresh)


def test_helper_preserve_attrs() -> None:
    reference_original = LITELLM_RESPONSES.ModelResponseStream(
        custom_attr=CUSTOM_VALUE, usage=USAGE_DATA
    )
    reference_target = LITELLM_RESPONSES.ModelResponseStream()
    ours_original = ModelResponseStream(custom_attr=CUSTOM_VALUE, usage=USAGE_DATA)
    ours_target = ModelResponseStream()
    preserve_upstream_non_openai_attributes(ours_target, ours_original)
    LITELLM_HELPERS.preserve_upstream_non_openai_attributes(
        reference_target, reference_original
    )
    reference_fields = fields_of(reference_target.model_dump())
    assert reference_fields[CUSTOM_ATTR] == CUSTOM_VALUE
    assert ours_target.model_dump()[CUSTOM_ATTR] == CUSTOM_VALUE
    assert "usage" not in reference_fields
    assert "usage" not in ours_target.model_dump()


def test_helper_safe_copy(monkeypatch: pytest.MonkeyPatch) -> None:
    span = datetime(2024, 1, 1, tzinfo=UTC)
    reference_input = copy_payload(span)
    ours_input = copy_payload(span)
    reference_copy = fields_of(LITELLM_HELPERS.safe_deep_copy(reference_input))
    ours_copy = fields_of(safe_deep_copy(ours_input))
    assert ours_copy == reference_copy
    copy_pairs = ((reference_copy, reference_input), (ours_copy, ours_input))
    for copied, original in copy_pairs:
        metadata = fields_of(copied[METADATA_KEY])
        assert metadata[PARENT_SPAN] == PLACEHOLDER
        assert metadata[MARKER_KEY] == MARKER_VALUE
        assert fields_of(copied[LITELLM_METADATA_KEY])[PARENT_SPAN] == PLACEHOLDER
        assert fields_of(original[METADATA_KEY])[PARENT_SPAN] is span
    reference_failed = failing_payload(span)
    ours_failed = failing_payload(span)
    reference_fallback = fields_of(LITELLM_HELPERS.safe_deep_copy(reference_failed))
    ours_fallback = fields_of(safe_deep_copy(ours_failed))
    fallback_pairs = ((reference_fallback, reference_failed), (ours_fallback, ours_failed))
    for fallback, original in fallback_pairs:
        assert fallback[METADATA_KEY] is original[METADATA_KEY]
        assert fields_of(fallback[METADATA_KEY])[PARENT_SPAN] is span
    reference_cycle = fields_of(LITELLM_HELPERS.safe_deep_copy(cyclic_payload()))
    ours_cycle = fields_of(safe_deep_copy(cyclic_payload()))
    for cycle in (reference_cycle, ours_cycle):
        assert fields_of(cycle["self"])["self"] is cycle["self"]
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "safe_memory_mode", SAFE_MEMORY_ON)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "safe_memory_mode", SAFE_MEMORY_ON)
    payload: dict[str, PythonValue] = {"a": {"b": 1}}
    assert safe_deep_copy(payload) is payload
    assert LITELLM_HELPERS.safe_deep_copy(payload) is payload


def test_helper_snapshot() -> None:
    span = datetime(2024, 1, 1, tzinfo=UTC)
    reference_input = copy_payload(span)
    ours_input = copy_payload(span)
    reference_snapshot = fields_of(LITELLM_HELPERS.independent_snapshot(reference_input))
    ours_snapshot = fields_of(independent_snapshot(ours_input))
    assert ours_snapshot == reference_snapshot
    snapshot_pairs = ((reference_snapshot, reference_input), (ours_snapshot, ours_input))
    for snapshot, original in snapshot_pairs:
        assert snapshot[METADATA_KEY] is not original[METADATA_KEY]
        assert fields_of(snapshot[METADATA_KEY])[PARENT_SPAN] is span
        assert fields_of(snapshot[LITELLM_METADATA_KEY])[PARENT_SPAN] is span
    reference_failed = failing_payload(span)
    ours_failed = failing_payload(span)
    reference_fallback = fields_of(LITELLM_HELPERS.independent_snapshot(reference_failed))
    ours_fallback = fields_of(independent_snapshot(ours_failed))
    fallback_pairs = ((reference_fallback, reference_failed), (ours_fallback, ours_failed))
    for fallback, original in fallback_pairs:
        assert fallback[METADATA_KEY] is original[METADATA_KEY]
        assert fields_of(fallback[METADATA_KEY])[PARENT_SPAN] is span
    reference_cycle = fields_of(LITELLM_HELPERS.independent_snapshot(cyclic_payload()))
    ours_cycle = fields_of(independent_snapshot(cyclic_payload()))
    for cycle in (reference_cycle, ours_cycle):
        assert fields_of(cycle["self"])["self"] is cycle["self"]


def test_helper_filter_exceptions() -> None:
    deep: dict[str, PythonValue] = {"a": {"b": {"c": ValueError(BOOM)}}}
    ours_deep = fields_of(filter_exceptions_from_params(copy.deepcopy(deep)))
    reference_deep = fields_of(
        LITELLM_HELPERS.filter_exceptions_from_params(copy.deepcopy(deep))
    )
    assert ours_deep == reference_deep == {"a": {"b": {}}}
    shallow: dict[str, PythonValue] = {"a": {"b": ValueError(BOOM)}}
    expected = LITELLM_HELPERS.filter_exceptions_from_params(copy.deepcopy(shallow), 2)
    assert filter_exceptions_from_params(shallow, 1) == {"a": shallow["a"]}
    assert filter_exceptions_from_params(shallow, 2) == expected == {"a": {}}
    at_zero = ValueError(BOOM)
    assert error_text(filter_exceptions_from_params(at_zero, 0)) == BOOM
    assert error_text(LITELLM_HELPERS.filter_exceptions_from_params(at_zero, 0)) == BOOM
    ours_filtered = fields_of(filter_exceptions_from_params(filter_payload()))
    reference_filtered = fields_of(
        LITELLM_HELPERS.filter_exceptions_from_params(filter_payload())
    )
    expected_filtered = {"class": ValueError, "list": [{}, 1]}
    assert ours_filtered == reference_filtered == expected_filtered


def test_helper_filter_internal() -> None:
    data: dict[str, JsonValue] = {
        "a": 1, "skip_mcp_handler": 2, "mcp_handler_context": 3,
        "_skip_mcp_handler": 4, "x": 5,
    }
    expected: dict[str, JsonValue] = {"a": 1}
    assert filter_internal_params(dict(data), {"x"}) == expected
    assert filter_internal_params(dict(data)) == {"a": 1, "x": 5}
    assert LITELLM_HELPERS.filter_internal_params(dict(data), {"x"}) == expected


def test_helper_reexport() -> None:
    assert frozenset(helpers_package.__all__) == HELPER_NAMES
    assert frozenset(reexported_helpers.__all__) == HELPER_NAMES
    assert map_finish_reason is parts_map_finish_reason
    assert set(dir(LITELLM_HELPERS)) >= HELPER_NAMES


def test_utils_names_same() -> None:
    assert frozenset(utils_module.__all__) == UTILS_NAMES
    assert utils_module.completion_cost is completion_cost
    assert utils_module.cost_per_token is cost_per_token
    assert utils_module.response_cost_calculator is response_cost_calculator
    assert utils_module.get_max_tokens is get_max_tokens
    assert utils_module.get_model_info is get_model_info
    assert utils_module.get_provider_info is get_provider_info
    assert utils_module.get_llm_provider is get_llm_provider
    mutation_generation = utils_module.get_model_cost_mutation_generation
    assert mutation_generation is get_model_cost_mutation_generation
    assert utils_module.create_pretrained_tokenizer is create_pretrained_tokenizer
    assert utils_module.create_tokenizer is create_tokenizer
    assert utils_module.decode is decode
    assert utils_module.encode is encode
    assert utils_module.token_counter is token_counter


def test_budget_lifecycle(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    patch_costs(monkeypatch)
    reference_dir = tmp_path / "reference"
    reference_dir.mkdir()
    ours_dir = tmp_path / "ours"
    ours_dir.mkdir()

    monkeypatch.chdir(reference_dir)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    reference_marker = set(threading.enumerate())
    assert reference.user_dict == {}
    assert reference.create_budget(BUDGET, ALICE) == {"total_budget": BUDGET}
    expected_bob: dict[str, JsonValue] = {
        "total_budget": BUDGET,
        "duration": DAYS_IN_A_WEEK,
        "created_at": CREATED_AT,
        "last_updated_at": CREATED_AT,
    }
    assert reference.create_budget(BUDGET, BOB, "weekly", CREATED_AT) == expected_bob
    reference.update_cost(ALICE, model=CHAT_MODEL, input_text="hi", output_text="yo")
    reference_completion = LITELLM_RESPONSES.ModelResponse(model=CHAT_MODEL)
    reference.update_cost(ALICE, completion_obj=reference_completion)
    reference_projected = reference.projected_cost(CHAT_MODEL, PROJECTED_MESSAGES, ALICE)
    reference_total = reference.get_total_budget(ALICE)
    reference_current = reference.get_current_cost(ALICE)
    reference_model = reference.get_model_cost(ALICE)
    reference_users = reference.get_users()
    reference_reset = reference.reset_cost(ALICE)
    join_saves(reference_marker)
    assert reference.save_data() == {"status": "success"}
    reference_text = Path(COST_FILE).read_text()
    reloaded_reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    assert reloaded_reference.user_dict == reference.user_dict

    monkeypatch.chdir(ours_dir)
    ours = BudgetManager(PROJECT)
    ours_marker = set(threading.enumerate())
    assert ours.user_dict == {}
    assert ours.create_budget(BUDGET, ALICE) == {"total_budget": BUDGET}
    assert ours.create_budget(BUDGET, BOB, "weekly", CREATED_AT) == expected_bob
    ours.update_cost(ALICE, model=CHAT_MODEL, input_text="hi", output_text="yo")
    ours_completion = ModelResponse(model=CHAT_MODEL)
    ours.update_cost(ALICE, completion_obj=ours_completion)
    assert ours.projected_cost(CHAT_MODEL, PROJECTED_MESSAGES, ALICE) == reference_projected
    assert ours.get_current_cost(BOB) == reference.get_current_cost(BOB)
    assert ours.get_model_cost(BOB) == reference.get_model_cost(BOB)
    assert ours.get_total_budget(ALICE) == reference_total
    assert ours.get_current_cost(ALICE) == reference_current
    assert ours.get_model_cost(ALICE) == reference_model
    assert ours.get_users() == reference_users
    for user in (ALICE, BOB, ZOE):
        assert ours.is_valid_user(user) == reference.is_valid_user(user)
    assert ours.reset_cost(ALICE) == reference_reset
    assert ours.user_dict == reference.user_dict
    join_saves(ours_marker)
    assert ours.save_data() == {"status": "success"}
    ours_text = Path(COST_FILE).read_text()

    assert ours_text == reference_text
    assert JSON_ADAPTER.validate_json(ours_text) == ours.user_dict
    reloaded_ours = BudgetManager(PROJECT)
    assert reloaded_ours.user_dict == ours.user_dict


def test_budget_duration_reset(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    patch_costs(monkeypatch)
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    for manager in (reference, ours):
        manager.create_budget(BUDGET, ALICE, "daily", CREATED_AT)
        manager.create_budget(BUDGET, BOB)
        manager.update_cost(ALICE, model=CHAT_MODEL, input_text="a", output_text="b")
        manager.update_cost(BOB, model=CHAT_MODEL, input_text="a", output_text="b")
    clock = OneSecondClock(CREATED_AT + HOURS_IN_A_DAY * 60 * 60 - 1)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(time, "time", clock)
    for manager in (reference, ours):
        manager.reset_on_duration(ALICE)
    assert ours.user_dict == reference.user_dict
    assert ours.user_dict[ALICE]["current_cost"] != ZERO_COST
    clock.now = CREATED_AT + HOURS_IN_A_DAY * 60 * 60
    for manager in (reference, ours):
        manager.update_budget_all_users()
    assert ours.user_dict == reference.user_dict
    alice = ours.user_dict[ALICE]
    assert alice["current_cost"] == ZERO_COST
    assert alice["model_cost"] == {}
    assert alice["last_updated_at"] == clock.now
    assert "duration" not in ours.user_dict[BOB]
    assert ours.user_dict[BOB]["current_cost"] != ZERO_COST
    join_saves(marker)


def test_budget_durations(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    named = (("daily", ALICE), ("weekly", BOB), ("monthly", CAROL), ("yearly", DAVE))
    for duration, user in named:
        expected = reference.create_budget(BUDGET, user, duration, CREATED_AT)
        assert ours.create_budget(BUDGET, user, duration, CREATED_AT) == expected
    join_saves(marker)
    assert ours.user_dict == reference.user_dict
    durations = [ours.user_dict[user]["duration"] for _duration, user in named]
    assert durations == [1, DAYS_IN_A_WEEK, DAYS_IN_A_MONTH, DAYS_IN_A_YEAR]


def test_budget_bad_duration(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    with pytest.raises(ValueError, match="duration needs to be one of"):
        reference.create_budget(BUDGET, ALICE, "hourly")
    with pytest.raises(ValueError, match="duration needs to be one of"):
        ours.create_budget(BUDGET, ALICE, "hourly")
    expected_record: dict[str, JsonValue] = {"total_budget": BUDGET}
    assert ours.user_dict == reference.user_dict == {ALICE: expected_record}


def test_budget_missing_input(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    with pytest.raises(ValueError, match="Either a chat completion object"):
        reference.update_cost(ALICE)
    with pytest.raises(ValueError, match="Either a chat completion object"):
        ours.update_cost(ALICE)


def test_budget_missing_model(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    patch_costs(monkeypatch)
    monkeypatch.chdir(tmp_path)
    ours = BudgetManager(PROJECT)
    ours.create_budget(BUDGET, ALICE)
    with pytest.raises(TypeError, match="model must be a string"):
        ours.update_cost(ALICE, completion_obj=ModelResponse())


def test_budget_unknown_client() -> None:
    with pytest.raises(ValueError, match="client_type must be"):
        BudgetManager(PROJECT, UNKNOWN_CLIENT)


def test_budget_hosted_load() -> None:
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = HOSTED_REPLY
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert ours.user_dict == reference.user_dict == HOSTED_USERS
        assert len(server.requests) == LOAD_REQUESTS
        for request in server.requests:
            assert request["path"] == GET_BUDGET
            assert request["body"] == {"project_name": PROJECT}
            assert request["content_type"] == JSON_TYPE


def test_budget_hosted_error() -> None:
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = ERROR_REPLY
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert ours.user_dict == reference.user_dict == {}
        assert len(server.requests) == LOAD_REQUESTS


def test_budget_hosted_save() -> None:
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = HOSTED_REPLY
        server.replies[SET_BUDGET] = SAVED_REPLY
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert reference.save_data() == SAVED_REPLY
        assert ours.save_data() == SAVED_REPLY
        saves = [request for request in server.requests if request["path"] == SET_BUDGET]
        assert len(saves) == SAVE_REQUESTS
        reference_save, ours_save = saves
        assert reference_save["body"] == ours_save["body"] == HOSTED_SAVE_BODY


def test_budget_background_save(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    reference_dir = tmp_path / "reference"
    reference_dir.mkdir()
    ours_dir = tmp_path / "ours"
    ours_dir.mkdir()
    expected_record: dict[str, JsonValue] = {
        "total_budget": BUDGET,
        "duration": 1,
        "created_at": CREATED_AT,
        "last_updated_at": CREATED_AT,
    }
    monkeypatch.chdir(reference_dir)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    reference_marker = set(threading.enumerate())
    assert reference.create_budget(BUDGET, ALICE, "daily", CREATED_AT) == expected_record
    join_saves(reference_marker)
    reference_text = Path(COST_FILE).read_text()
    monkeypatch.chdir(ours_dir)
    ours = BudgetManager(PROJECT)
    ours_marker = set(threading.enumerate())
    assert ours.create_budget(BUDGET, ALICE, "daily", CREATED_AT) == expected_record
    join_saves(ours_marker)
    assert Path(COST_FILE).read_text() == reference_text
    assert JSON_ADAPTER.validate_json(reference_text) == {ALICE: expected_record}


def test_budget_verbose(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    monkeypatch.chdir(tmp_path)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "set_verbose", True)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "set_verbose", True)
    with caplog.at_level(logging.INFO):
        ours = BudgetManager(PROJECT)
        reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    assert ours.user_dict == reference.user_dict == {}
    messages = [record.getMessage() for record in caplog.records]
    assert messages == [
        NOT_FOUND_MESSAGE,
        LOCAL_DICT_MESSAGE,
        NOT_FOUND_MESSAGE,
        LOCAL_DICT_MESSAGE,
    ]
