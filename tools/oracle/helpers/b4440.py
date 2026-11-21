import asyncio
import json
import re
import sys
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from importlib import import_module
from types import ModuleType
from typing import Literal, Protocol, TypedDict

import litellm
from litellm.exceptions import BadRequestError as BadRequest, ContentPolicyViolationError as ContentPolicy
from litellm.exceptions import ContextWindowExceededError as ContextWindow, UnsupportedParamsError as Unsupported
from litellm.litellm_core_utils import health_check_utils
from litellm.litellm_core_utils.health_check_helpers import HealthCheckHelpers
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ImageResponse as ImageReply, LlmProviders as Providers
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

litellm.suppress_debug_info = True

REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
METADATA: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
ERRORS: TypeAdapter[list[dict[str, JsonValue]]] = TypeAdapter(list[dict[str, JsonValue]])
DROPPED = ("api_key", "litellm_logging_obj", "file", "image")
TRACKED = ("tags", "user_api_key", "user_api_key_hash", "user_api_key_alias", "user_api_key_team_id",
           "user_api_key_team_alias", "user_api_key_spend")
CHEAPEST: list[str] = []
Mode = Literal[
    "chat", "completion", "embedding", "audio_speech", "audio_transcription", "image_generation",
    "image_edit", "video_generation", "rerank", "realtime", "batch", "responses", "ocr", "evaluation",
]


class Failure(TypedDict):
    class_name: str
    message: str


class ModeStep(TypedDict):
    mode: Mode
    model: str
    custom_llm_provider: str
    params: dict[str, JsonValue]
    prompt: str | None
    inputs: list[str] | None
    logging: bool
    failure: Failure | None


class WildcardStep(TypedDict):
    model: str
    custom_llm_provider: str
    params: dict[str, JsonValue]
    cheapest: list[str]


class Input(TypedDict):
    mode: list[ModeStep]
    wildcard: list[WildcardStep]
    tracking: list[dict[str, dict[str, JsonValue]]]
    response: list[dict[str, dict[str, str]]]


class UpdateFromKwargs(Protocol):
    def update_from_kwargs(self, **params: JsonValue) -> None: ...


class LoggingRecorder(UpdateFromKwargs):
    def __init__(self) -> None:
        self.updates: list[JsonValue] = []

    @override
    def update_from_kwargs(self, **params: JsonValue) -> None:
        self.updates.append(project(params))


class Recorder(ModuleType):
    def __init__(self, name: str, calls: list[JsonValue], failure: Exception | None) -> None:
        super().__init__(name)
        self.calls = calls
        self.failure = failure

    def note(self, name: str, params: dict[str, JsonValue]) -> dict[str, JsonValue]:
        self.calls.append({"name": name, "arguments": project(params)})
        if self.failure is not None:
            raise self.failure
        return {"status": "ok"}


class CallsModule(Recorder):
    def __init__(self, calls: list[JsonValue], failure: Exception | None) -> None:
        super().__init__("litellm", calls, failure)
        self.ImageResponse = ImageReply
        self.BadRequestError = BadRequest
        self.ContentPolicyViolationError = ContentPolicy
        self.LlmProviders = Providers

    async def acompletion(self, **params: JsonValue) -> JsonValue:
        return self.note("acompletion", params)

    async def atext_completion(self, **params: JsonValue) -> JsonValue:
        return self.note("atext_completion", params)

    async def aembedding(self, **params: JsonValue) -> JsonValue:
        return self.note("aembedding", params)

    async def aspeech(self, **params: JsonValue) -> JsonValue:
        return self.note("aspeech", params)

    async def atranscription(self, **params: JsonValue) -> JsonValue:
        return self.note("atranscription", params)

    async def aimage_generation(self, **params: JsonValue) -> JsonValue:
        return self.note("aimage_generation", params)

    async def aimage_edit(self, **params: JsonValue) -> JsonValue:
        return self.note("aimage_edit", params)

    async def avideo_generation(self, **params: JsonValue) -> JsonValue:
        return self.note("avideo_generation", params)

    async def arerank(self, **params: JsonValue) -> JsonValue:
        return self.note("arerank", params)

    async def aresponses(self, **params: JsonValue) -> JsonValue:
        return self.note("aresponses", params)

    async def asystemone(self, **params: JsonValue) -> JsonValue:
        return self.note("asystemone", params)

    async def alist_batches(self, **params: JsonValue) -> JsonValue:
        return self.note("alist_batches", params)

    async def aocr(self, **params: JsonValue) -> JsonValue:
        return self.note("aocr", params)


class RealtimeModule(Recorder):
    async def _realtime_health_check(self, **params: JsonValue) -> JsonValue:
        return self.note("realtime_health_check", params)


def cheapest_models(custom_llm_provider: str, n: int = 3) -> list[str]:
    _ = custom_llm_provider
    return CHEAPEST[:n]


class RequestUtils(ModuleType):
    def __init__(self) -> None:
        super().__init__("litellm.litellm_core_utils.llm_request_utils")

    def __getattr__(self, name: str) -> Callable[[str, int], list[str]]:
        _ = name
        return cheapest_models


GET_HANDLERS: Callable[..., dict[Mode, Callable[[], Awaitable[JsonValue]]]] = HealthCheckHelpers.get_mode_handlers  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L164-L287
WILDCARD: Callable[..., Awaitable[dict[str, JsonValue]]] = HealthCheckHelpers.ahealth_check_wildcard_models  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L57-L85
TRACK: Callable[..., dict[str, JsonValue]] = HealthCheckHelpers._update_model_params_with_health_check_tracking_information  # noqa: SLF001  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L87-L110
CREATE_RESPONSE: Callable[..., dict[str, JsonValue]] = health_check_utils._create_health_check_response  # noqa: SLF001  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_utils.py#L19-L30


def scripted_failure(failure: Failure) -> Exception:
    class_name = failure["class_name"]
    if class_name == "content_policy":
        return ContentPolicy(failure["message"], "model", "openai")
    if class_name == "bad_request":
        return BadRequest(failure["message"], "model", "openai")
    if class_name == "context_window":
        return ContextWindow(failure["message"], "model", "openai")
    if class_name == "unsupported_params":
        return Unsupported(failure["message"], "model", "openai")
    return RuntimeError(failure["message"])


def model_json(value: BaseModel) -> JsonValue:
    text = value.model_dump_json()
    return REPLY.validate_json(text)


def clean(value: JsonValue | BaseModel) -> JsonValue:
    match value:
        case BaseModel():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L277-L287
            return model_json(value)
        case dict(): return {key: clean(item) for key, item in value.items() if key not in DROPPED}
        case list(): return [clean(item) for item in value]
        case str() | bool() | int() | float() | None:
            return value


def project(params: JsonValue) -> JsonValue:
    text = json.dumps(clean(params), default=model_json)
    return REPLY.validate_json(text)


def logging_object() -> Logging:
    return Logging(
        model="health", messages=[], stream=False, call_type="acompletion",
        litellm_call_id="b4440", function_id="b4440", start_time=datetime.now(UTC),
    )


def swap(name: str, module: ModuleType) -> ModuleType:
    saved = import_module(name)
    sys.modules[name] = module
    return saved


def repeated_key(message: str) -> str:
    found = re.search(r"multiple values for keyword argument '([^']+)'", message)
    if found is None:
        return message
    return found.group(1)


def payload_field(error: ValidationError) -> str:
    match ERRORS.validate_json(error.json())[0].get("loc"):
        case [str() as field, *_]:
            return field
        case None | bool() | int() | float() | str() | dict() | list():
            return ""


async def mode_step(step: ModeStep) -> JsonValue:
    calls: list[JsonValue] = []
    failure = None if step["failure"] is None else scripted_failure(step["failure"])
    logging = LoggingRecorder()
    params: dict[str, JsonValue | LoggingRecorder] = {
        key: value for key, value in step["params"].items()
    }
    if step["logging"]:
        params["litellm_logging_obj"] = logging
    saved_litellm = swap("litellm", CallsModule(calls, failure))
    saved_realtime = swap("litellm.realtime_api.main", RealtimeModule("litellm.realtime_api.main", calls, failure))
    try:
        handlers = GET_HANDLERS(
            step["model"], step["custom_llm_provider"], params, step["prompt"], step["inputs"]
        )
        try:
            _ = await handlers[step["mode"]]()
        except TypeError as error:
            return {"error": {"kind": "repeated", "detail": repeated_key(str(error))}}
        except ValidationError as error:
            return {"error": {"kind": "payload", "detail": payload_field(error)}}
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L164-L287
            detail = "" if step["failure"] is None else step["failure"]["class_name"]
            return {"error": {"kind": "backend", "detail": detail}}
        call = calls[0] if calls else None
        update = logging.updates[0] if logging.updates else None
        result = "empty" if step["failure"] is not None else "ok"
        return {"call": call, "logging": update, "result": result}
    finally:
        sys.modules["litellm"] = saved_litellm
        sys.modules["litellm.realtime_api.main"] = saved_realtime


async def wildcard_step(step: WildcardStep) -> JsonValue:
    calls: list[JsonValue] = []
    CHEAPEST[:] = step["cheapest"]
    previous_litellm = swap("litellm", CallsModule(calls, None))
    previous_utils = swap("litellm.litellm_core_utils.llm_request_utils", RequestUtils())
    try:
        try:
            _ = await WILDCARD(
                step["model"], step["custom_llm_provider"], step["params"], logging_object()
            )
        except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L57-L85
            return {"error": {"kind": "no_model", "detail": str(error)}}
        return {"call": calls[0] if calls else None}
    finally:
        sys.modules["litellm"] = previous_litellm
        sys.modules["litellm.litellm_core_utils.llm_request_utils"] = previous_utils


def tracking_step(step: dict[str, dict[str, JsonValue]]) -> JsonValue:
    tracked = TRACK(dict(step["params"]))
    metadata = METADATA.validate_json(json.dumps(tracked["litellm_metadata"], default=model_json))
    answer: dict[str, JsonValue] = {key: metadata.get(key) for key in TRACKED}
    return answer


async def run_steps(input_value: Input) -> list[JsonValue]:
    results: list[JsonValue] = [await mode_step(step) for step in input_value["mode"]]
    wildcards: list[JsonValue] = [await wildcard_step(step) for step in input_value["wildcard"]]
    results.extend(wildcards)
    results.extend(tracking_step(step) for step in input_value["tracking"])
    results.extend(CREATE_RESPONSE(step["headers"]) for step in input_value["response"])
    return results


def run(input_value: Input) -> JsonValue:
    results = asyncio.run(run_steps(input_value))
    return results
