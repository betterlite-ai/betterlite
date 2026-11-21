import asyncio
import json
import re
import sys
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from types import ModuleType
from typing import Literal, Protocol, TypedDict

import litellm
import litellm.litellm_core_utils.llm_request_utils
import litellm.realtime_api.main
from litellm.exceptions import BadRequestError as BadRequest
from litellm.exceptions import ContentPolicyViolationError as ContentPolicy
from litellm.exceptions import ContextWindowExceededError as ContextWindow
from litellm.exceptions import UnsupportedParamsError as Unsupported
from litellm.litellm_core_utils import health_check_utils
from litellm.litellm_core_utils.health_check_helpers import HealthCheckHelpers
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ImageResponse as ImageReply
from litellm.types.utils import LlmProviders as Providers
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

litellm.suppress_debug_info = True

REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
METADATA: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
CHEAPEST: list[str] = []
DROPPED = ("api_key", "litellm_logging_obj", "file", "image")
TRACKED = (
    "tags",
    "user_api_key",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_team_alias",
    "user_api_key_spend",
)

Mode = Literal[
    "chat", "completion", "embedding", "audio_speech", "audio_transcription",
    "image_generation", "image_edit", "video_generation", "rerank", "realtime",
    "batch", "responses", "ocr", "evaluation",
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


class TrackingStep(TypedDict):
    params: dict[str, JsonValue]


class ResponseStep(TypedDict):
    headers: dict[str, str]


class Input(TypedDict):
    mode: list[ModeStep]
    wildcard: list[WildcardStep]
    tracking: list[TrackingStep]
    response: list[ResponseStep]


class LoggingRecorder:
    def __init__(self) -> None:
        self.updates: list[JsonValue] = []

    def update_from_kwargs(self, **params: JsonValue) -> None:
        self.updates.append(project(params))


class Recorder(ModuleType):
    calls: list[JsonValue]
    failure: Exception | None

    def __init__(self, name: str, calls: list[JsonValue], failure: Exception | None) -> None:
        super().__init__(name)
        self.calls = calls
        self.failure = failure

    def note(self, name: str, params: dict[str, JsonValue]) -> JsonValue:
        entry = REPLY.validate_python({"name": name, "arguments": project(params)})
        self.calls.append(entry)
        if self.failure is not None:
            raise self.failure
        return REPLY.validate_python({"status": "ok"})


class CallsModule(Recorder):
    ImageResponse: type[ImageReply]
    BadRequestError: type[BadRequest]
    ContentPolicyViolationError: type[ContentPolicy]
    LlmProviders: type[Providers]

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


class RequestUtils(ModuleType):
    def __init__(self) -> None:
        super().__init__("litellm.litellm_core_utils.llm_request_utils")

    @staticmethod
    def pick_cheapest_chat_models_from_llm_provider(
        custom_llm_provider: str, n: int = 3
    ) -> list[str]:
        _ = custom_llm_provider
        return list(CHEAPEST)[:n]


class GetHandlers(Protocol):
    def __call__(
        self,
        model: str,
        custom_llm_provider: str,
        model_params: dict[str, JsonValue | LoggingRecorder],
        prompt: str | None,
        inputs: list[str] | None,
        /,
    ) -> dict[Mode, Callable[[], Awaitable[JsonValue]]]: ...


class WildcardProbe(Protocol):
    async def __call__(
        self,
        model: str,
        custom_llm_provider: str,
        model_params: dict[str, JsonValue],
        litellm_logging_obj: Logging,
        /,
    ) -> dict[str, JsonValue]: ...


class TrackParams(Protocol):
    def __call__(self, model_params: dict[str, JsonValue], /) -> dict[str, JsonValue]: ...


class CreateResponse(Protocol):
    def __call__(self, response_headers: dict[str, str], /) -> dict[str, JsonValue]: ...


GET_HANDLERS: GetHandlers = HealthCheckHelpers.get_mode_handlers
WILDCARD: WildcardProbe = HealthCheckHelpers.ahealth_check_wildcard_models
TRACK: TrackParams = HealthCheckHelpers._update_model_params_with_health_check_tracking_information  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L87-L110
CREATE_RESPONSE: CreateResponse = health_check_utils._create_health_check_response  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_utils.py#L19-L30


def project(params: JsonValue) -> JsonValue:
    text = json.dumps(clean(params), default=model_json)
    return REPLY.validate_json(text)


def clean(value: JsonValue | BaseModel) -> JsonValue:
    match value:
        case BaseModel():
            return model_json(value)
        case dict():
            return {key: clean(item) for key, item in value.items() if key not in DROPPED}
        case list():
            return [clean(item) for item in value]
        case str() | bool() | int() | float() | None:
            return value


def model_json(value: BaseModel) -> JsonValue:
    return REPLY.validate_json(value.model_dump_json())


def logging_object() -> Logging:
    return Logging(
        model="health",
        messages=[],
        stream=False,
        call_type="acompletion",
        litellm_call_id="b4440",
        function_id="b4440",
        start_time=datetime.now(UTC),
    )


def swap(name: str, module: ModuleType) -> ModuleType:
    saved = sys.modules[name]
    sys.modules[name] = module
    return saved


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


def repeated_key(message: str) -> str:
    found = re.search(r"multiple values for keyword argument '([^']+)'", message)
    if found is None:
        return message
    return found.group(1)


def payload_field(error: ValidationError) -> str:
    first = error.errors()[0]
    return str(first["loc"][0])


def backend_error(step: ModeStep) -> JsonValue:
    failure = step["failure"]
    detail = "" if failure is None else failure["class_name"]
    return REPLY.validate_python({"error": {"kind": "backend", "detail": detail}})


def success_outcome(
    step: ModeStep, calls: list[JsonValue], logging: LoggingRecorder
) -> JsonValue:
    call = calls[0] if calls else None
    update = logging.updates[0] if logging.updates else None
    result = "empty" if step["failure"] is not None else "ok"
    return REPLY.validate_python({"call": call, "logging": update, "result": result})


async def mode_step(step: ModeStep) -> JsonValue:
    calls: list[JsonValue] = []
    failure = None if step["failure"] is None else scripted_failure(step["failure"])
    params: dict[str, JsonValue | LoggingRecorder] = {}
    for key, value in step["params"].items():
        params[key] = value
    logging = LoggingRecorder()
    if step["logging"]:
        params["litellm_logging_obj"] = logging
    calls_module = CallsModule(calls, failure)
    realtime_module = RealtimeModule("litellm.realtime_api.main", calls, failure)
    previous_litellm = swap("litellm", calls_module)
    previous_realtime = swap("litellm.realtime_api.main", realtime_module)
    try:
        handlers = GET_HANDLERS(
            step["model"],
            step["custom_llm_provider"],
            params,
            step["prompt"],
            step["inputs"],
        )
        handler = handlers[step["mode"]]
        try:
            _ = await handler()
        except TypeError as error:
            detail = repeated_key(str(error))
            return REPLY.validate_python({"error": {"kind": "repeated", "detail": detail}})
        except ValidationError as error:
            detail = payload_field(error)
            return REPLY.validate_python({"error": {"kind": "payload", "detail": detail}})
        except Exception:  # noqa: BLE001, E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L164-L287
            return backend_error(step)
        return success_outcome(step, calls, logging)
    finally:
        sys.modules["litellm"] = previous_litellm
        sys.modules["litellm.realtime_api.main"] = previous_realtime


async def wildcard_step(step: WildcardStep) -> JsonValue:
    calls: list[JsonValue] = []
    CHEAPEST[:] = step["cheapest"]
    module = CallsModule(calls, None)
    utils = RequestUtils()
    previous_litellm = swap("litellm", module)
    previous_utils = swap("litellm.litellm_core_utils.llm_request_utils", utils)
    try:
        try:
            _ = await WILDCARD(
                step["model"], step["custom_llm_provider"], step["params"], logging_object()
            )
        except Exception as error:  # noqa: BLE001, E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/health_check_helpers.py#L57-L85
            detail = str(error)
            return REPLY.validate_python({"error": {"kind": "no_model", "detail": detail}})
        return REPLY.validate_python({"call": calls[0] if calls else None})
    finally:
        sys.modules["litellm"] = previous_litellm
        sys.modules["litellm.litellm_core_utils.llm_request_utils"] = previous_utils


def tracking_step(step: TrackingStep) -> JsonValue:
    tracked = TRACK(dict(step["params"]))
    metadata = METADATA.validate_json(json.dumps(tracked["litellm_metadata"], default=model_json))
    return REPLY.validate_python({key: metadata.get(key) for key in TRACKED})


def response_step(step: ResponseStep) -> JsonValue:
    return REPLY.validate_python(CREATE_RESPONSE(step["headers"]))


async def run_steps(input_value: Input) -> list[JsonValue]:
    results: list[JsonValue] = []
    for step in input_value["mode"]:
        results.append(await mode_step(step))
    for step in input_value["wildcard"]:
        results.append(await wildcard_step(step))
    for step in input_value["tracking"]:
        results.append(tracking_step(step))
    for step in input_value["response"]:
        results.append(response_step(step))
    return results


def run(input_value: Input) -> JsonValue:
    return REPLY.validate_python(asyncio.run(run_steps(input_value)))
