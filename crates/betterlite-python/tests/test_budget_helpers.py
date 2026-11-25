import copy
import importlib
import json
import logging
import threading
import time
from collections.abc import Callable
from datetime import UTC, datetime
from enum import IntEnum
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import ModuleType, TracebackType
from typing import ClassVar, Final, Protocol, Self

import httpx
import httpx2
import pytest
from pydantic import ConfigDict, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

import betterlite
import betterlite._core_helpers as helpers_package
import betterlite._model_info as model_info
import betterlite.litellm_core_utils.core_helpers as reexported_helpers
from betterlite import utils as utils_module
from betterlite._core_helpers import (
    add_missing_spend_metadata_to_litellm_metadata,
    coerce_token_limit,
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
    is_expected_client_error,
    map_finish_reason,
    max_retries_per_request_hit,
    normalize_drop_params,
    preserve_upstream_non_openai_attributes,
    process_response_headers,
    proxy_stamped_used_client_oauth_token,
    reconstruct_model_name,
    remove_index_from_tool_calls,
    remove_items_at_indices,
    safe_deep_copy,
    safe_divide,
    safe_divide_seconds,
    set_provider_response_headers_in_hidden_params,
    set_response_cost_in_hidden_params,
)
from betterlite._core_helpers.values import PythonValue, object_fields, text_of as values_text_of
from betterlite._cost import completion_cost, cost_per_token, response_cost_calculator
from betterlite._model_info import get_max_tokens, get_model_info, get_provider_info
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
    ModuleClient,
    module_level_client,
)
from betterlite.types.utils.choices import map_finish_reason as parts_map_finish_reason
from betterlite.types.utils.response import ModelResponse, ModelResponseStream

JSON_TYPE: Final = "application/json"
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
CHAT_MODEL: Final = "gpt-4o"
BUDGET: Final = 10.0
ZERO_COST: Final = 0.0
TENTH_COST: Final = 0.1
UPDATE_COST: Final = 0.2
TOTAL_COST: Final = 0.3
INPUT_TEXT: Final = "hi"
OUTPUT_TEXT: Final = "later"
ALICE: Final = "alice"
BOB: Final = "bob"
HOSTED: Final = "hosted"
GET_BUDGET: Final = "/get_budget"
SET_BUDGET: Final = "/set_budget"
MOVED_GET: Final = "/moved"
AGENT_VALUE: Final = "b166-agent/1"
STAMPED: Final = "stamped-token"
METADATA_STAMPED: Final = "metadata-token"
REQUEST_TIMEOUT: Final = 600.0
CONNECT_TIMEOUT: Final = 5.0
BLOCK_SECONDS: Final = 10.0

HELPER_NAMES: Final[frozenset[str]] = frozenset({
        "add_missing_spend_metadata_to_litellm_metadata", "coerce_token_limit",
        "drop_params_env_flag", "drop_params_flag", "filter_exceptions_from_params",
        "filter_internal_params", "get_litellm_metadata_from_kwargs",
        "get_metadata_variable_name_from_kwargs", "get_or_create_metadata_bucket",
        "get_provider_response_headers_from_hidden_params", "independent_snapshot",
        "is_codex_user_agent", "is_expected_client_error", "map_finish_reason",
        "max_retries_per_request_hit", "normalize_drop_params",
        "preserve_upstream_non_openai_attributes", "process_response_headers",
        "proxy_stamped_used_client_oauth_token", "reconstruct_model_name",
        "remove_index_from_tool_calls", "remove_items_at_indices", "safe_deep_copy",
        "safe_divide", "safe_divide_seconds",
        "set_provider_response_headers_in_hidden_params", "set_response_cost_in_hidden_params",
})
UTILS_NAMES: Final[frozenset[str]] = frozenset({
        "completion_cost", "cost_per_token", "create_pretrained_tokenizer", "create_tokenizer",
        "decode", "encode", "get_llm_provider", "get_max_tokens",
        "get_model_cost_mutation_generation", "get_model_info", "get_provider_info",
        "response_cost_calculator", "token_counter",
})
NORMALIZE_VALUES: Final[tuple[PythonValue, ...]] = (
    None, True, "true", " TRUE ", "on", "1", 1, 0, "banana", 2, {}, "false", "off",
)
ENVIRONMENTS: Final[tuple[dict[str, str], ...]] = (
    {}, {"LITELLM_DROP_PARAMS": "true"}, {"LITELLM_DROP_PARAMS": "  "},
    {"LITELLM_DROP_PARAMS": "banana"}, {"LITELLM_DROP_PARAMS": "0"},
    {"LITELLM_DROP_PARAMS": "off"}, {"LITELLM_DROP_PARAMS": "FALSE"})
CODEX_AGENTS: Final[tuple[str, ...]] = ("codex-cli/1.0", "Codex_v", "codex /x", "xcodex", "openai")
FINISH_REASONS: Final[tuple[str, ...]] = (
    "stop", "length", "tool_calls", "content_filtered", "error", "weird",
)
HEADER_CASES: Final[tuple[dict[str, str], ...]] = (
    {"content-type": "text/plain", "llm_provider-x": "y", "x-litellm-z": "w", "b": "c"},
    {"x-ratelimit-remaining-requests": "9"}, {})
DIVIDE_CASES: Final[tuple[tuple[float, float, float], ...]] = (
    (4.0, 2.0, 0.0), (4.0, 0.0, 0.0), (4.0, 0.0, 7.0), (4.0, -2.0, 0.0))
SECONDS_CASES: Final[tuple[tuple[float, float, float | None], ...]] = (
    (10.0, 0.0, None), (10.0, 0.0, 5.0), (10.0, -1.0, None), (10.0, 4.0, None))
NAME_CASES: Final[tuple[tuple[str, str | None, dict[str, PythonValue]], ...]] = (
    ("claude", "bedrock", {}), ("claude", "anthropic", {}),
    ("bedrock/x", "bedrock", {"deployment": "anthropic/y"}),
    ("x", "bedrock", {"deployment": "claude"}), ("", "bedrock", {}), ("x", None, {}))
META_KWARGS_CASES: Final[tuple[dict[str, PythonValue], ...]] = (
    {}, {"litellm_params": None}, {"litellm_params": {}}, {"litellm_params": {METADATA_KEY: {}}},
    {"litellm_params": {LITELLM_METADATA_KEY: {}}},
    {"litellm_params": {METADATA_KEY: None, LITELLM_METADATA_KEY: {"m": 2}}},
    {"litellm_params": {METADATA_KEY: {"user_api_key": "k"}, LITELLM_METADATA_KEY: {"m": 2}}},
    {"litellm_params": {METADATA_KEY: {"user_api_key": "k"}}})
TOKEN_LIMITS: Final[tuple[PythonValue, ...]] = (
    True, False, 128000, 0, -5, 12.9, "128000", "128,000", "", None, [1])


class RetryCount(IntEnum):
    TWO = 2


RETRY_KWARGS: Final[tuple[dict[str, PythonValue], ...]] = (
    {}, {METADATA_KEY: {}}, {METADATA_KEY: {"request_retry_count": 2}},
    {METADATA_KEY: {"request_retry_count": True}}, {METADATA_KEY: {"request_retry_count": "2"}},
    {METADATA_KEY: {"request_retry_count": 2}, LITELLM_METADATA_KEY: {"request_retry_count": 5}},
    {METADATA_KEY: {"request_retry_count": RetryCount.TWO}})
ERROR_CASES: Final[tuple[tuple[PythonValue, PythonValue, PythonValue], ...]] = (
    (None, None, None), (None, 400, None), (None, 499, None), (None, 500, None),
    (None, True, None), (None, "429", None), (None, "nope", None), (None, 404.5, None),
    (None, 429, "openai"), ("401", 200, None), ("500", 200, None), (503, 200, None),
    (None, None, "openai"))
PROJECTED_MESSAGES: Final[list[dict[str, JsonValue]]] = [
    {"role": "user", "content": "ab"}, {"role": "user", "content": "c"}]
USAGE_DATA: Final[dict[str, JsonValue]] = {
    "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
EXCEPTION_ADAPTER: Final[TypeAdapter[BaseException]] = TypeAdapter(
    BaseException, config=ConfigDict(arbitrary_types_allowed=True)
)
HOSTED_USERS: Final[dict[str, dict[str, JsonValue]]] = {ALICE: {"total_budget": BUDGET}}
HOSTED_DATA: Final[JsonValue] = JSON_ADAPTER.validate_python(HOSTED_USERS)
HOSTED_REPLY: Final[dict[str, JsonValue]] = {"status": "ok", "data": HOSTED_DATA}
ERROR_REPLY: Final[dict[str, JsonValue]] = {"status": "error"}
BAD_DATA_REPLY: Final[dict[str, JsonValue]] = {"status": "ok", "data": {ALICE: 5}}
SAVED_REPLY: Final[dict[str, JsonValue]] = {"status": "success"}
SAVE_BODY: Final[dict[str, JsonValue]] = {"project_name": PROJECT, "user_dict": HOSTED_DATA}


class PrivateBag(Protocol):
    __pydantic_private__: dict[str, PythonValue] | None


class ReferenceResponse(PrivateBag, Protocol):
    def model_dump(self, **kwargs: PythonValue) -> PythonValue: ...


class ReferenceCall(Protocol):
    def __call__(
        self,
        *args: PythonValue | ReferenceResponse | logging.Logger | httpx2.Headers | set[str],
        **kwargs: PythonValue | ReferenceResponse | logging.Logger | httpx2.Headers | set[str],
    ) -> PythonValue: ...


class CallModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class ResponseModule(Protocol):
    def __getattr__(self, name: str) -> Callable[..., ReferenceResponse]: ...


class BudgetShape(Protocol):
    user_dict: dict[str, dict[str, JsonValue]]
    def __getattr__(self, name: str) -> ReferenceCall: ...


class BudgetModule(Protocol):
    def __getattr__(self, name: str) -> Callable[..., BudgetShape]: ...


LITELLM: ModuleType = importlib.import_module("litellm")
BETTERLITE: ModuleType = betterlite
LITELLM_HELPERS: CallModule = importlib.import_module("litellm.litellm_core_utils.core_helpers")
LITELLM_HTTP: CallModule = importlib.import_module("litellm.llms.custom_httpx.http_handler")
LITELLM_ERRORS: CallModule = importlib.import_module("litellm.exceptions")
LITELLM_RESPONSES: ResponseModule = importlib.import_module("litellm")
LITELLM_BUDGET: BudgetModule = importlib.import_module("litellm.budget_manager")


class Router(ModelResponse):
    ...


def helper_function() -> None:
    ...


class OneSecondClock:
    def __init__(self, now: float) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


class StatusError(Exception):
    def __init__(
        self,
        code: PythonValue = None,
        status_code: PythonValue = None,
        llm_provider: PythonValue = None,
    ) -> None:
        super().__init__(BOOM)
        self.code: PythonValue = code
        self.status_code: PythonValue = status_code
        self.llm_provider: PythonValue = llm_provider


class GatedSaver:
    def __init__(self) -> None:
        self.started: threading.Event = threading.Event()
        self.release: threading.Event = threading.Event()
        self.finished: threading.Event = threading.Event()
        self.workers: list[threading.Thread] = []

    def __call__(self) -> JsonValue:
        self.workers.append(threading.current_thread())
        self.started.set()
        self.release.wait(timeout=BLOCK_SECONDS)
        self.finished.set()
        return {"status": "success"}


class WriteGate:
    now: ClassVar[int] = 0
    peak: ClassVar[int] = 0


def gated_write_text(path: Path, text: str) -> int:
    WriteGate.now += 1
    WriteGate.peak = max(WriteGate.peak, WriteGate.now)
    time.sleep(0.1)
    WriteGate.now -= 1
    with path.open("w") as handle:
        return handle.write(text)


def text_of(value: JsonValue) -> str:
    text = values_text_of(value)
    if text is None:
        raise TypeError("expected a string")
    return text


def as_error(value: PythonValue) -> BaseException:
    try:
        return EXCEPTION_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise TypeError("expected an exception") from error


def fake_token_counter(
    model: str = "", text: str | None = None, messages: list[dict[str, JsonValue]] | None = None
) -> int:
    if text is not None:
        return len(model) + len(text)
    if messages is not None:
        return len(model) + sum(len(text_of(message["content"])) for message in messages)
    return len(model)


def fake_cost_per_token(
    *, model: str, prompt_tokens: int, completion_tokens: int
) -> tuple[float, float]:
    prompt_cost = len(model) / 8 + prompt_tokens / 64
    return (prompt_cost, completion_tokens / 64)


def fake_completion_cost(*, completion_response: PythonValue) -> float:
    if completion_response is None:
        return 0.0
    return 0.5


def fractional_completion_cost(*, completion_response: PythonValue) -> float:
    if completion_response is None:
        return 0.0
    return UPDATE_COST


def patch_costs(monkeypatch: pytest.MonkeyPatch) -> None:
    for module in (BETTERLITE, LITELLM):
        # ast-grep-ignore: no-string-mocking
        monkeypatch.setattr(module, "token_counter", fake_token_counter)
        # ast-grep-ignore: no-string-mocking
        monkeypatch.setattr(module, "cost_per_token", fake_cost_per_token)
        # ast-grep-ignore: no-string-mocking
        monkeypatch.setattr(module, "completion_cost", fake_completion_cost)


def fields_of(value: PythonValue) -> dict[str, PythonValue]:
    fields = object_fields(value)
    if fields is None:
        raise TypeError("expected an object")
    return fields


def error_text(value: PythonValue) -> str:
    return str(as_error(value))


def hidden_of(response: PrivateBag) -> dict[str, PythonValue]:
    private = response.__pydantic_private__
    if private is None:
        return {}
    return fields_of(private["_hidden_params"])


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
    router = Router.model_construct(id="r", created=0, object="r", choices=[])
    logging_object = Router.model_construct(id="l", created=0, object="l", choices=[])
    return {
        "class": ValueError,
        "function": helper_function,
        "error": ValueError(BOOM),
        "none": None,
        "router": router,
        "logging": logging_object,
        "list": [ValueError(BOOM), {"router": router}, 1],
    }


def join_saves(marker: set[threading.Thread]) -> None:
    for thread in set(threading.enumerate()) - marker:
        if not thread.daemon:
            thread.join(timeout=10)


class RecordingServer(ThreadingHTTPServer):
    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), RecordingHandler)
        self.replies: dict[str, JsonValue] = {}
        self.statuses: dict[str, int] = {}
        self.locations: dict[str, str] = {}
        self.requests: list[dict[str, JsonValue]] = []
        RecordingHandler.owner = self
        self.thread: threading.Thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

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
            "path": self.path, "content_type": self.headers.get("Content-Type"),
            "agent": self.headers.get("User-Agent"), "cookie": self.headers.get("Cookie"),
            "body": parsed,
        }
        self.owner.requests.append(recorded)
        location = self.owner.locations.get(self.path)
        status = self.owner.statuses.get(self.path)
        if status is None:
            status = 200 if location is None else 307
        payload = json.dumps(self.owner.replies[self.path]).encode() if location is None else b""
        self.send_response(status)
        if location is not None:
            self.send_header("Location", location)
        self.send_header("content-type", JSON_TYPE)
        self.send_header("content-length", str(len(payload)))
        self.send_header("set-cookie", "session=b166")
        self.end_headers()
        self.wfile.write(payload)


def test_helper_scalars_and_flags() -> None:
    for agent in CODEX_AGENTS:
        assert is_codex_user_agent(agent) == LITELLM_HELPERS.is_codex_user_agent(agent)
    for numerator, denominator, default in DIVIDE_CASES:
        expected = LITELLM_HELPERS.safe_divide(numerator, denominator, default)
        assert safe_divide(numerator, denominator, default) == expected
    for seconds, denominator, optional_default in SECONDS_CASES:
        expected = LITELLM_HELPERS.safe_divide_seconds(seconds, denominator, optional_default)
        assert safe_divide_seconds(seconds, denominator, optional_default) == expected
    logger = logging.getLogger("b166-drop")
    for value in NORMALIZE_VALUES:
        assert normalize_drop_params(value) == LITELLM_HELPERS.normalize_drop_params(value)
        expected = LITELLM_HELPERS.drop_params_flag(value, "TEST_FLAG", logger)
        assert drop_params_flag(value, "TEST_FLAG", logger) == expected
    for environ in ENVIRONMENTS:
        expected = LITELLM_HELPERS.drop_params_env_flag(environ, logger)
        assert drop_params_env_flag(environ, logger) == expected


def test_helper_messages_metadata() -> None:
    for reason in FINISH_REASONS:
        assert map_finish_reason(reason) == LITELLM_HELPERS.map_finish_reason(reason)
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
    reference_items: list[JsonValue] = ["a", "b", "c", "d"]
    ours_items: list[JsonValue] = list(reference_items)
    LITELLM_HELPERS.remove_items_at_indices(reference_items, [0, 1, 9, 1, -1])
    remove_items_at_indices(ours_items, [0, 1, 9, 1, -1])
    assert ours_items == reference_items == ["c", "d"]
    remove_items_at_indices(None, [0])
    cases: tuple[dict[str, PythonValue], ...] = (
        {}, {METADATA_KEY: {}}, {LITELLM_METADATA_KEY: None}, {LITELLM_METADATA_KEY: {}},
    )
    for kwargs in cases:
        expected = LITELLM_HELPERS.get_metadata_variable_name_from_kwargs(kwargs)
        assert get_metadata_variable_name_from_kwargs(kwargs) == expected
    ours_request: dict[str, PythonValue] = {}
    reference_request: dict[str, PythonValue] = {}
    expected = LITELLM_HELPERS.get_or_create_metadata_bucket(reference_request)
    assert get_or_create_metadata_bucket(ours_request) == expected
    assert ours_request == reference_request == {METADATA_KEY: {}}
    existing: dict[str, PythonValue] = {METADATA_KEY: {}}
    key, bucket = get_or_create_metadata_bucket(existing)
    assert key == METADATA_KEY
    assert bucket is existing[METADATA_KEY]
    for kwargs in META_KWARGS_CASES:
        expected = LITELLM_HELPERS.get_litellm_metadata_from_kwargs(copy.deepcopy(kwargs))
        assert get_litellm_metadata_from_kwargs(copy.deepcopy(kwargs)) == expected
    for model_name, provider, metadata in NAME_CASES:
        expected = LITELLM_HELPERS.reconstruct_model_name(model_name, provider, metadata)
        assert reconstruct_model_name(model_name, provider, metadata) == expected
    for bad in (5, helper_function):
        with pytest.raises(TypeError, match="deployment must be a string"):
            reconstruct_model_name("x", "bedrock", {"deployment": bad})


def test_helper_headers() -> None:
    for headers in HEADER_CASES:
        expected = LITELLM_HELPERS.process_response_headers(headers)
        assert process_response_headers(headers) == expected
        preserved = LITELLM_HELPERS.process_response_headers(
            headers, preserve_litellm_internal_headers=True)
        assert process_response_headers(
            headers, preserve_litellm_internal_headers=True) == preserved
    raw_headers = httpx2.Headers({"content-type": "text/plain", "x-litellm-z": "w"})
    raw_expected = LITELLM_HELPERS.process_response_headers(
        raw_headers, preserve_litellm_internal_headers=True)
    assert process_response_headers(
        raw_headers, preserve_litellm_internal_headers=True) == raw_expected
    headers = {"A": "b", "llm_provider-x": "y", "x-litellm-z": "w"}
    span = datetime(2024, 1, 1, tzinfo=UTC)
    reference = LITELLM_RESPONSES.ModelResponse()
    ours = ModelResponse()
    hidden_of(reference)["live"] = span
    hidden_of(ours)["live"] = span
    set_response_cost_in_hidden_params(ours, 0.25)
    LITELLM_HELPERS.set_response_cost_in_hidden_params(reference, 0.25)
    assert hidden_of(ours) == hidden_of(reference)
    set_provider_response_headers_in_hidden_params(ours, headers)
    LITELLM_HELPERS.set_provider_response_headers_in_hidden_params(reference, headers)
    assert hidden_of(ours) == hidden_of(reference)
    expected = LITELLM_HELPERS.get_provider_response_headers_from_hidden_params(reference)
    assert get_provider_response_headers_from_hidden_params(ours) == expected == headers
    assert get_provider_response_headers_from_hidden_params(None) is None
    fresh = ModelResponse()
    set_response_cost_in_hidden_params(fresh, None)
    assert "additional_headers" not in hidden_of(fresh)
    assert get_provider_response_headers_from_hidden_params(fresh) is None


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


def test_helper_copies(monkeypatch: pytest.MonkeyPatch) -> None:
    span = datetime(2024, 1, 1, tzinfo=UTC)
    copies: list[tuple[dict[str, PythonValue], dict[str, PythonValue]]] = []
    snapshots: list[tuple[dict[str, PythonValue], dict[str, PythonValue]]] = []
    for deep_copier in (LITELLM_HELPERS.safe_deep_copy, safe_deep_copy):
        original = copy_payload(span)
        copies.append((fields_of(deep_copier(original)), original))
    for snapshotter in (LITELLM_HELPERS.independent_snapshot, independent_snapshot):
        original = copy_payload(span)
        snapshots.append((fields_of(snapshotter(original)), original))
    assert copies[0][0] == copies[1][0]
    assert snapshots[0][0] == snapshots[1][0]
    for copied, original in copies:
        assert fields_of(copied[METADATA_KEY])[PARENT_SPAN] == PLACEHOLDER
        assert fields_of(copied[METADATA_KEY])[MARKER_KEY] == MARKER_VALUE
        assert fields_of(copied[LITELLM_METADATA_KEY])[PARENT_SPAN] == PLACEHOLDER
        assert fields_of(original[METADATA_KEY])[PARENT_SPAN] is span
    for snapshot, original in snapshots:
        assert snapshot[METADATA_KEY] is not original[METADATA_KEY]
        assert fields_of(snapshot[METADATA_KEY])[PARENT_SPAN] is span
        assert fields_of(snapshot[LITELLM_METADATA_KEY])[PARENT_SPAN] is span
    for fallback_copier in (LITELLM_HELPERS.safe_deep_copy, safe_deep_copy, independent_snapshot):
        failed = failing_payload(span)
        fallback = fields_of(fallback_copier(failed))
        assert fallback[METADATA_KEY] is failed[METADATA_KEY]
        assert fields_of(fallback[METADATA_KEY])[PARENT_SPAN] is span
        cycle = fields_of(fallback_copier(cyclic_payload()))
        assert fields_of(cycle["self"])["self"] is cycle["self"]
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "safe_memory_mode", True)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "safe_memory_mode", True)
    payload: dict[str, PythonValue] = {"a": {"b": 1}}
    assert safe_deep_copy(payload) is payload
    assert LITELLM_HELPERS.safe_deep_copy(payload) is payload


def test_helper_filters() -> None:
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
    expected_filtered: dict[str, PythonValue] = {"class": ValueError, "list": [{}, 1]}
    assert ours_filtered == reference_filtered == expected_filtered
    ours_cycle = cyclic_payload()
    reference_cycle = cyclic_payload()
    assert fields_of(filter_exceptions_from_params(ours_cycle, 1))["self"] is ours_cycle
    reference_cycled = LITELLM_HELPERS.filter_exceptions_from_params(reference_cycle, 1)
    assert fields_of(reference_cycled)["self"] is reference_cycle
    branching: dict[str, PythonValue] = {}
    branching["a"] = branching
    branching["b"] = branching
    branching["c"] = branching
    spread = fields_of(filter_exceptions_from_params(branching))
    assert spread["a"] is spread["b"] is spread["c"] is branching
    data: dict[str, JsonValue] = {
        "a": 1, "skip_mcp_handler": 2, "mcp_handler_context": 3, "_skip_mcp_handler": 4, "x": 5,
    }
    expected_internal: dict[str, JsonValue] = {"a": 1}
    assert filter_internal_params(dict(data), {"x"}) == expected_internal
    assert filter_internal_params(dict(data)) == {"a": 1, "x": 5}
    assert LITELLM_HELPERS.filter_internal_params(dict(data), {"x"}) == expected_internal


def test_helper_expected_error() -> None:
    for code, status_code, llm_provider in ERROR_CASES:
        ours = StatusError(code, status_code, llm_provider)
        reference = StatusError(code, status_code, llm_provider)
        expected = LITELLM_HELPERS.is_expected_client_error(reference)
        assert is_expected_client_error(ours) == expected
    for error in (
        LITELLM_ERRORS.BudgetExceededError(current_cost=1.0, max_budget=0.0),
        LITELLM_ERRORS.BadRequestError(message=BOOM, model=CHAT_MODEL, llm_provider="openai"),
        LITELLM_ERRORS.RateLimitError(message=BOOM, llm_provider="openai", model=CHAT_MODEL),
        LITELLM_ERRORS.APIConnectionError(message=BOOM, llm_provider="openai", model=CHAT_MODEL),
    ):
        expected = LITELLM_HELPERS.is_expected_client_error(as_error(error))
        assert is_expected_client_error(as_error(error)) == expected
    assert is_expected_client_error(None) is False


def test_helper_request_guards() -> None:
    for value in TOKEN_LIMITS:
        assert coerce_token_limit(value) == LITELLM_HELPERS.coerce_token_limit(value)
    for kwargs in RETRY_KWARGS:
        for limit in (None, 0, 2, 3):
            expected = LITELLM_HELPERS.max_retries_per_request_hit(kwargs, limit)
            assert max_retries_per_request_hit(kwargs, limit) == expected
    spend: dict[str, PythonValue] = {"user_api_key": "k", "x": 1}
    litellm_metadata: dict[str, PythonValue] = {"m": 2}
    expected = LITELLM_HELPERS.add_missing_spend_metadata_to_litellm_metadata(
        {"m": 2}, {"user_api_key": "k", "x": 1}
    )
    spent = add_missing_spend_metadata_to_litellm_metadata(litellm_metadata, spend)
    assert spent == expected == {"m": 2, "user_api_key": "k"}
    assert spent is litellm_metadata
    params: dict[str, PythonValue] = {LITELLM_METADATA_KEY: {"used_client_oauth_token": STAMPED}}
    metadata: dict[str, PythonValue] = {"used_client_oauth_token": METADATA_STAMPED}
    expected = LITELLM_HELPERS.proxy_stamped_used_client_oauth_token(
        {"used_client_oauth_token": METADATA_STAMPED},
        {LITELLM_METADATA_KEY: {"used_client_oauth_token": STAMPED}},
    )
    assert proxy_stamped_used_client_oauth_token(metadata, params) == expected == STAMPED
    assert proxy_stamped_used_client_oauth_token({}, params) == STAMPED
    expected = LITELLM_HELPERS.proxy_stamped_used_client_oauth_token(metadata, None)
    assert proxy_stamped_used_client_oauth_token(metadata, None) == expected == METADATA_STAMPED
    assert proxy_stamped_used_client_oauth_token(None, None) is None


def test_helper_reexport_and_utils() -> None:
    assert frozenset(helpers_package.__all__) == HELPER_NAMES
    assert frozenset(reexported_helpers.__all__) == HELPER_NAMES
    assert map_finish_reason is parts_map_finish_reason
    assert set(dir(LITELLM_HELPERS)) >= HELPER_NAMES
    assert frozenset(utils_module.__all__) == UTILS_NAMES
    assert utils_module.completion_cost is completion_cost
    assert utils_module.cost_per_token is cost_per_token
    assert utils_module.response_cost_calculator is response_cost_calculator
    assert utils_module.get_max_tokens is get_max_tokens
    assert utils_module.get_model_info is get_model_info
    assert utils_module.get_provider_info is get_provider_info
    assert utils_module.get_llm_provider is get_llm_provider
    mutation_generation = model_info.get_model_cost_mutation_generation
    assert utils_module.get_model_cost_mutation_generation is mutation_generation
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
    assert reference.create_budget(BUDGET, ALICE) == {"total_budget": BUDGET}
    expected_bob: dict[str, JsonValue] = {
        "total_budget": BUDGET, "duration": DAYS_IN_A_WEEK,
        "created_at": CREATED_AT, "last_updated_at": CREATED_AT,
    }
    assert reference.create_budget(BUDGET, BOB, "weekly", CREATED_AT) == expected_bob
    reference.update_cost(ALICE, model=CHAT_MODEL, input_text=INPUT_TEXT, output_text=OUTPUT_TEXT)
    reference.update_cost(ALICE, completion_obj=LITELLM_RESPONSES.ModelResponse(model=CHAT_MODEL))
    reference_projected = reference.projected_cost(CHAT_MODEL, PROJECTED_MESSAGES, ALICE)
    reference_reset = reference.reset_cost(ALICE)
    join_saves(reference_marker)
    assert reference.save_data() == {"status": "success"}
    reference_text = Path("user_cost.json").read_text()
    monkeypatch.chdir(ours_dir)
    ours = BudgetManager(PROJECT)
    ours_marker = set(threading.enumerate())
    assert ours.user_dict == {}
    assert ours.create_budget(BUDGET, ALICE) == {"total_budget": BUDGET}
    assert ours.create_budget(BUDGET, BOB, "weekly", CREATED_AT) == expected_bob
    ours.update_cost(ALICE, model=CHAT_MODEL, input_text=INPUT_TEXT, output_text=OUTPUT_TEXT)
    ours.update_cost(ALICE, completion_obj=ModelResponse(model=CHAT_MODEL))
    assert ours.projected_cost(CHAT_MODEL, PROJECTED_MESSAGES, ALICE) == reference_projected
    assert ours.get_current_cost(ALICE) == reference.get_current_cost(ALICE)
    assert ours.get_model_cost(ALICE) == reference.get_model_cost(ALICE)
    assert ours.get_total_budget(ALICE) == reference.get_total_budget(ALICE)
    for user in (ALICE, BOB, "zoe"):
        assert ours.is_valid_user(user) == reference.is_valid_user(user)
    assert ours.reset_cost(ALICE) == reference_reset
    join_saves(ours_marker)
    assert ours.save_data() == {"status": "success"}
    assert Path("user_cost.json").read_text() == reference_text
    reloaded = BudgetManager(PROJECT)
    assert reloaded.user_dict == ours.user_dict


def test_budget_duration_reset(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    patch_costs(monkeypatch)
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    for manager in (reference, ours):
        manager.create_budget(BUDGET, ALICE, "daily", CREATED_AT)
        manager.create_budget(BUDGET, BOB)
        manager.update_cost(ALICE, model=CHAT_MODEL, input_text=INPUT_TEXT, output_text=OUTPUT_TEXT)
        manager.update_cost(BOB, model=CHAT_MODEL, input_text=INPUT_TEXT, output_text=OUTPUT_TEXT)
    clock = OneSecondClock(CREATED_AT + HOURS_IN_A_DAY * 60 * 60 - 1)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(time, "time", clock)
    for manager in (reference, ours):
        manager.reset_on_duration(ALICE)
    assert ours.get_current_cost(ALICE) == reference.get_current_cost(ALICE) != ZERO_COST
    clock.now = CREATED_AT + HOURS_IN_A_DAY * 60 * 60
    for manager in (reference, ours):
        manager.update_budget_all_users()
    assert ours.get_current_cost(ALICE) == reference.get_current_cost(ALICE) == ZERO_COST
    assert ours.get_model_cost(ALICE) == reference.get_model_cost(ALICE) == {}
    alice = ours.user_dict[ALICE]
    assert alice["last_updated_at"] == clock.now
    assert "duration" not in ours.user_dict[BOB]
    assert ours.get_current_cost(BOB) == reference.get_current_cost(BOB) != ZERO_COST
    join_saves(marker)


def test_budget_durations(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    named = (("daily", ALICE), ("weekly", BOB), ("monthly", "carol"), ("yearly", "dave"))
    for duration, user in named:
        expected = reference.create_budget(BUDGET, user, duration, CREATED_AT)
        assert ours.create_budget(BUDGET, user, duration, CREATED_AT) == expected
    join_saves(marker)
    durations = [ours.user_dict[user]["duration"] for _duration, user in named]
    assert durations == [1, DAYS_IN_A_WEEK, DAYS_IN_A_MONTH, DAYS_IN_A_YEAR]
    with pytest.raises(ValueError, match="duration needs to be one of"):
        reference.create_budget(BUDGET, "zoe", "hourly")
    with pytest.raises(ValueError, match="duration needs to be one of"):
        ours.create_budget(BUDGET, "zoe", "hourly")
    assert ours.get_total_budget("zoe") == reference.get_total_budget("zoe") == BUDGET
    assert ours.get_current_cost("zoe") == reference.get_current_cost("zoe") == ZERO_COST


def test_budget_missing_input(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    patch_costs(monkeypatch)
    monkeypatch.chdir(tmp_path)
    reference = LITELLM_BUDGET.BudgetManager(PROJECT)
    ours = BudgetManager(PROJECT)
    with pytest.raises(ValueError, match="Either a chat completion object"):
        reference.update_cost(ALICE)
    with pytest.raises(ValueError, match="Either a chat completion object"):
        ours.update_cost(ALICE)
    with pytest.raises(TypeError, match="model must be a string"):
        ours.update_cost(ALICE, completion_obj=ModelResponse())
    with pytest.raises(ValueError, match="client_type must be"):
        BudgetManager(PROJECT, "remote")


def test_budget_bad_stored_data(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    corrupt: dict[str, JsonValue] = {ALICE: 5}
    Path("user_cost.json").write_text(json.dumps(corrupt))
    with pytest.raises(ValidationError, match=ALICE):
        BudgetManager(PROJECT)
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = BAD_DATA_REPLY
        with pytest.raises(ValidationError, match=ALICE):
            BudgetManager(PROJECT, HOSTED, server.url)


def test_budget_decimal_costs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    stored: dict[str, JsonValue] = {ALICE: {"total_budget": BUDGET, "current_cost": TENTH_COST}}
    Path("user_cost.json").write_text(json.dumps(stored))
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "completion_cost", fractional_completion_cost)
    manager = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    manager.update_cost(ALICE, completion_obj=ModelResponse(model=CHAT_MODEL))
    assert manager.get_current_cost(ALICE) == TOTAL_COST
    assert manager.get_model_cost(ALICE) == {CHAT_MODEL: UPDATE_COST}
    join_saves(marker)


def test_budget_hosted_load() -> None:
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = HOSTED_REPLY
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert ours.get_users() == reference.get_users() == [ALICE]
        assert ours.get_total_budget(ALICE) == reference.get_total_budget(ALICE) == BUDGET
        assert ours.get_current_cost(ALICE) == reference.get_current_cost(ALICE) == ZERO_COST
        assert server.requests
        for request in server.requests:
            assert request["path"] == GET_BUDGET
            assert request["body"] == {"project_name": PROJECT}
            assert request["content_type"] == JSON_TYPE
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = ERROR_REPLY
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert ours.get_users() == reference.get_users() == []
        assert server.requests


def test_budget_hosted_http_errors() -> None:
    for status in (401, 500):
        with RecordingServer() as server:
            server.replies[GET_BUDGET] = ERROR_REPLY
            server.statuses[GET_BUDGET] = status
            with pytest.raises(httpx.HTTPStatusError) as reference_raised:
                LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
            assert reference_raised.value.response.status_code == status
            with pytest.raises(httpx2.HTTPStatusError) as ours_raised:
                BudgetManager(PROJECT, HOSTED, server.url)
            assert ours_raised.value.response.status_code == status


def test_budget_hosted_transport(monkeypatch: pytest.MonkeyPatch) -> None:
    assert module_level_client.timeout.read == REQUEST_TIMEOUT
    assert module_level_client.timeout.connect == CONNECT_TIMEOUT
    assert module_level_client.client.follow_redirects
    assert module_level_client.ssl_verify
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "ssl_verify", False)
    monkeypatch.setenv("LITELLM_USER_AGENT", AGENT_VALUE)
    configured = ModuleClient()
    assert configured.ssl_verify is False
    assert configured.client.headers["user-agent"] == AGENT_VALUE
    configured.client.close()
    reference_headers = fields_of(LITELLM_HTTP.get_default_headers())
    assert reference_headers["User-Agent"] == AGENT_VALUE
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = HOSTED_REPLY
        server.replies[MOVED_GET] = HOSTED_REPLY
        server.replies[SET_BUDGET] = SAVED_REPLY
        server.locations[GET_BUDGET] = MOVED_GET
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert ours.get_total_budget(ALICE) == reference.get_total_budget(ALICE) == BUDGET
        assert reference.save_data() == SAVED_REPLY
        assert ours.save_data() == SAVED_REPLY
    requests = list(server.requests)
    expected_paths = [GET_BUDGET, MOVED_GET, GET_BUDGET, MOVED_GET, SET_BUDGET, SET_BUDGET]
    assert [request["path"] for request in requests] == expected_paths
    agents = [text_of(request["agent"]) for request in requests]
    ours_agent = module_level_client.client.headers["user-agent"]
    assert agents == [agents[0], agents[0], ours_agent, ours_agent, agents[0], ours_agent]
    for request in requests:
        assert request["cookie"] is None


def test_budget_hosted_save() -> None:
    with RecordingServer() as server:
        server.replies[GET_BUDGET] = HOSTED_REPLY
        server.replies[SET_BUDGET] = SAVED_REPLY
        reference = LITELLM_BUDGET.BudgetManager(PROJECT, HOSTED, server.url)
        ours = BudgetManager(PROJECT, HOSTED, server.url)
        assert reference.save_data() == SAVED_REPLY
        assert ours.save_data() == SAVED_REPLY
        saves = [request for request in server.requests if request["path"] == SET_BUDGET]
        reference_save, ours_save = saves
        assert reference_save["body"] == ours_save["body"] == SAVE_BODY


def test_budget_background_save(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    manager = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    saver = GatedSaver()
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(manager, "save_data", saver)
    manager.create_budget(BUDGET, ALICE, "daily", CREATED_AT)
    assert saver.started.wait(timeout=BLOCK_SECONDS)
    assert not saver.release.is_set()
    assert not saver.finished.is_set()
    assert saver.workers[0] is not threading.current_thread()
    saver.release.set()
    join_saves(marker)
    assert saver.finished.is_set()


def test_budget_saves_serialized(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    manager = BudgetManager(PROJECT)
    manager.create_budget(BUDGET, ALICE)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(Path, "write_text", gated_write_text)
    threads = [threading.Thread(target=manager.save_data) for _ in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=BLOCK_SECONDS)
    assert WriteGate.peak == 1
    assert not any(thread.is_alive() for thread in threads)
    assert not Path("user_cost.json.pending").exists()
    saved = fields_of(JSON_ADAPTER.validate_json(Path("user_cost.json").read_text()))
    assert saved == {ALICE: {"total_budget": BUDGET}}


def test_budget_verbose(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    monkeypatch.chdir(tmp_path)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(BETTERLITE, "set_verbose", True)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(LITELLM, "set_verbose", True)
    with caplog.at_level(logging.INFO):
        BudgetManager(PROJECT)
        LITELLM_BUDGET.BudgetManager(PROJECT)
    messages = [record.getMessage() for record in caplog.records]
    assert messages == ["User Dictionary not found!", "user dict from local: {}"] * 2


def test_budget_default_created_at(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    manager = BudgetManager(PROJECT)
    marker = set(threading.enumerate())
    first = manager.create_budget(BUDGET, ALICE, "daily")
    second = manager.create_budget(BUDGET, BOB, "daily")
    assert first["created_at"] == second["created_at"] == first["last_updated_at"]
    join_saves(marker)
