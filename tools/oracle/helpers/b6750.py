import asyncio
import io
import re
from collections.abc import Awaitable, Callable
from contextlib import redirect_stdout
from typing import TypedDict

import litellm
from litellm.litellm_core_utils import llm_request_utils
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue, TypeAdapter

CHEAPEST: list[str] = []
DROPPED = ("api_key", "litellm_logging_obj", "litellm_metadata")
RAW_KEYS = ("raw_request_api_base", "raw_request_body", "raw_request_headers", "error")
STACK_MARKERS = ("\nstack trace: ", "\nstacktrace: ")
MISSING_PROVIDER = (
    "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. "
    "You passed model={model}"
)
UUID4 = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
TEXT: TypeAdapter[str] = TypeAdapter(str)
FLAG: TypeAdapter[bool] = TypeAdapter(bool)
MESSAGES: TypeAdapter[list[JsonValue] | None] = TypeAdapter(list[JsonValue] | None)
RECORD: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
RESULT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])

# The three litellm.a* attributes the four probed modes call, typed as the async calls a health check
# makes; the wildcard helper imports acompletion itself, so the attribute carries the fake too.
HEALTH_CHECK: Callable[[dict[str, JsonValue], str | None, str | None, list[str] | None], Awaitable[dict[str, JsonValue]]] = litellm.main.ahealth_check  # noqa: E501  # pyright: ignore[reportAssignmentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L8725-L8831


class ResolutionAnswer(TypedDict):
    model: str
    provider: str


class RawRequest(TypedDict, total=False):
    raw_request_api_base: str | None
    raw_request_body: JsonValue
    raw_request_headers: JsonValue
    error: str | None


class EntryStep(TypedDict):
    params: dict[str, JsonValue]
    mode: str | None
    prompt: str | None
    inputs: list[str] | None
    catalog: dict[str, str | None]
    resolution: ResolutionAnswer | None
    cheapest: list[str]
    headers: dict[str, str] | None
    failure: str | None
    raw_request: RawRequest | None


class Input(TypedDict):
    steps: list[EntryStep]


# The provider resolver ahealth_check calls: it answers the scripted resolution and records what it asked,
# or raises the missing-provider error B174's ResolutionError gives.
class ScriptedResolution:
    def __init__(self, answer: ResolutionAnswer | None) -> None:
        self.answer = answer
        self.requested: dict[str, JsonValue] | None = None

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
        **_kwargs: JsonValue,
    ) -> tuple[str, str, str | None, str | None]:
        self.requested = {
            "model": model,
            "custom_llm_provider": custom_llm_provider,
            "api_base": api_base,
            "api_key": api_key,
        }
        if self.answer is None:
            raise RuntimeError(MISSING_PROVIDER.format(model=model))
        return (self.answer["model"], self.answer["provider"], None, None)


class ProbeReply:
    def __init__(self, headers: dict[str, str] | None) -> None:
        self._hidden_params: dict[str, JsonValue] = {} if headers is None else {"headers": headers}


class ProbeRecorder:
    def __init__(
        self,
        headers: dict[str, str] | None,
        failure: str | None,
        raw_request: RawRequest | None,
    ) -> None:
        self.headers = headers
        self.failure = failure
        self.raw_request = raw_request
        self.arguments: dict[str, JsonValue] | None = None
        self.logging: Logging | None = None

    async def acompletion(
        self, *, litellm_logging_obj: Logging | None = None, **params: JsonValue
    ) -> ProbeReply:
        self.note("acompletion", litellm_logging_obj, params)
        return ProbeReply(self.headers)

    async def aembedding(
        self, *, litellm_logging_obj: Logging | None = None, **params: JsonValue
    ) -> ProbeReply:
        self.note("aembedding", litellm_logging_obj, params)
        return ProbeReply(self.headers)

    async def atext_completion(
        self, *, litellm_logging_obj: Logging | None = None, **params: JsonValue
    ) -> ProbeReply:
        self.note("atext_completion", litellm_logging_obj, params)
        return ProbeReply(self.headers)

    def note(
        self, name: str, logging_obj: Logging | None, params: dict[str, JsonValue]
    ) -> None:
        self.logging = logging_obj
        kept = {key: params[key] for key in params if key not in DROPPED}
        recorded: dict[str, JsonValue] = {"name": name, "arguments": kept}
        self.arguments = recorded
        self.write_raw_request(logging_obj)
        if self.failure is not None:
            raise RuntimeError(self.failure)

    def write_raw_request(self, logging_obj: Logging | None) -> None:
        if logging_obj is None or self.raw_request is None:
            return
        record = RECORD.validate_python(self.raw_request)
        raw = {key: record.get(key) for key in RAW_KEYS}
        logging_obj.model_call_details.update({"raw_request_typed_dict": raw})  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L1360-L1400


def scripted_cost(catalog: dict[str, str | None]) -> dict[str, dict[str, str]]:
    return {
        model: {} if mode is None else {"mode": mode}
        for model, mode in catalog.items()
    }


def cheapest_models(custom_llm_provider: str, n: int = 1) -> list[str]:
    _ = custom_llm_provider
    return CHEAPEST[:n]


def split_error(error: str) -> tuple[str, str | None]:
    for marker in STACK_MARKERS:
        if marker in error:
            head, stack = error.split(marker, 1)
            return head, stack
    return error, None


def raw_request_view(result: dict[str, JsonValue]) -> JsonValue:
    raw = result.get("raw_request_typed_dict")
    if raw is None:
        return None
    record = RECORD.validate_python(raw)
    return {key: record.get(key) for key in RAW_KEYS}


def outcome_view(result: dict[str, JsonValue]) -> JsonValue:
    error = result.get("error")
    if error is None:
        return {"kind": "healthy", "headers": result}
    head, stack = split_error(TEXT.validate_python(error))
    return {
        "kind": "failed",
        "error_head": head,
        "has_stack": stack is not None,
        "stack_length": 0 if stack is None else len(stack),
        "raw_request": raw_request_view(result),
    }


def logging_view(logging_obj: Logging | None) -> JsonValue:
    if logging_obj is None:
        return None
    return {
        "model": logging_obj.model,
        "messages": MESSAGES.validate_python(logging_obj.messages),  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L609-L619
        "stream": FLAG.validate_python(logging_obj.stream),  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L609-L619
        "call_type": TEXT.validate_python(logging_obj.call_type),  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L609-L619
        "log_raw_request_response": logging_obj.log_raw_request_response,
        "raw_request_only": logging_obj.raw_request_only,
        "call_id_uuid4": UUID4.match(logging_obj.litellm_call_id) is not None,
        "function_id_uuid4": UUID4.match(logging_obj.function_id) is not None,
        "trace_id_uuid4": UUID4.match(logging_obj.litellm_trace_id) is not None,
    }


async def step_view(step: EntryStep) -> JsonValue:
    resolution = ScriptedResolution(step["resolution"])
    recorder = ProbeRecorder(step["headers"], step["failure"], step["raw_request"])
    CHEAPEST[:] = step["cheapest"]
    litellm.main.get_llm_provider = resolution  # noqa: E501  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L8770-L8777
    litellm.acompletion = recorder.acompletion
    litellm.aembedding = recorder.aembedding
    litellm.atext_completion = recorder.atext_completion
    llm_request_utils.pick_cheapest_chat_models_from_llm_provider = cheapest_models
    litellm.model_cost = scripted_cost(step["catalog"])
    params = dict(step["params"])
    answer = await HEALTH_CHECK(params, step["mode"], step["prompt"], step["inputs"])
    # LiteLLM returns the caught exception object under "exception", which no JSON result can carry
    carried = {key: answer[key] for key in answer if key != "exception"}
    result = RESULT.validate_python(carried)
    return {
        "resolution": resolution.requested,
        "probe": recorder.arguments,
        "logging": logging_view(recorder.logging),
        "outcome": outcome_view(result),
    }


def run(input_value: Input) -> JsonValue:
    with redirect_stdout(io.StringIO()):
        views: list[JsonValue] = []
        for step in input_value["steps"]:
            views.append(asyncio.run(step_view(step)))
        return views
