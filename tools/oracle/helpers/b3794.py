import asyncio
import logging
import os
from typing import Final, Literal, Protocol, TypedDict, cast

import httpx
from fastapi import HTTPException
from litellm.caching.dual_cache import DualCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.repelloai import repelloai
from litellm.proxy.guardrails.guardrail_hooks.repelloai.repelloai import RepelloAIGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import CallTypesLiteral
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

BODY_ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

CONTENT_TYPE_HEADER: Final = "content-type"
API_KEY_HEADER: Final = "x-api-key"
LOGGER_NAME: Final = "LiteLLM Proxy"
CLEARED_ENVIRONMENT: Final = (
    "ARGUS_API_KEY",
    "REPELLOAI_API_KEY",
    "REPELLOAI_API_BASE",
)

Mode = Literal["init", "pre_call"]


class Answer(TypedDict):
    status: int
    body: dict[str, JsonValue]


class RequestRecord(TypedDict):
    path: str
    api_key_present: bool
    content_type: str | None
    body: JsonValue


class Caught(TypedDict):
    status: int | None
    detail: JsonValue


class RecordedInfo(TypedDict):
    guardrail_status: str


class HookMetadata(TypedDict, total=False):
    applied_guardrails: list[str]
    standard_logging_guardrail_information: list[RecordedInfo]


class HookData(TypedDict, total=False):
    messages: list[dict[str, str]]
    metadata: HookMetadata
    litellm_metadata: HookMetadata


class Output(TypedDict):
    created: bool
    api_base: str | None
    asset_id: str | None
    key_source: str | None
    event_hooks: list[str]
    requests: list[RequestRecord]
    applied: list[str]
    status: str | None
    caught: Caught | None
    warned: bool


class Guardrail(Protocol):
    async_handler: AsyncHTTPHandler
    api_base: str
    asset_id: str | None
    repelloai_api_key: str

    async def async_pre_call_hook(
        self,
        *,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: HookData,
        call_type: CallTypesLiteral,
    ) -> Exception | str | dict[str, JsonValue] | None: ...

    @classmethod
    def get_supported_event_hooks(cls) -> list[GuardrailEventHooks]: ...


class Input(TypedDict):
    mode: Mode
    env: dict[str, str]
    api_key: str | None
    api_base: str | None
    asset_id: str | None
    unreachable_fallback: str | None
    data: HookData
    answer: Answer | None


class Script:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.calls: list[RequestRecord] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "path": request.url.path,
                "api_key_present": API_KEY_HEADER in request.headers,
                "content_type": request.headers.get(CONTENT_TYPE_HEADER),
                "body": BODY_ADAPTER.validate_json(request.content),
            }
        )
        return httpx.Response(self.answer["status"], json=self.answer["body"], request=request)


class WarningCounter(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.warned = False

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.WARNING:
            self.warned = True


def fallback(value: str | None) -> Literal["fail_closed", "fail_open"]:
    if value == "fail_open":
        return "fail_open"
    return "fail_closed"


def build_guardrail(call: Input) -> Guardrail:
    for name in CLEARED_ENVIRONMENT:
        if name in os.environ:
            del os.environ[name]
    os.environ.update(call["env"])
    return cast(Guardrail, RepelloAIGuardrail(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/repelloai/repelloai.py#L142-L182

        guardrail_name="repello",
        api_key=call["api_key"],
        api_base=call["api_base"],
        asset_id=call["asset_id"],
        unreachable_fallback=fallback(call["unreachable_fallback"]),
        event_hook=GuardrailEventHooks.pre_call,
        default_on=True,
    ))


def key_source(call: Input, key: str) -> str:
    explicit = call["api_key"]
    if explicit is not None and explicit != "" and explicit == key:
        return "explicit"
    argus = call["env"].get("ARGUS_API_KEY")
    if argus is not None and argus != "" and argus == key:
        return "argus"
    return "repelloai"


def event_hooks() -> list[str]:
    return [hook.name for hook in RepelloAIGuardrail.get_supported_event_hooks()]


def metadata_bucket(data: HookData) -> HookMetadata | None:
    litellm_metadata = data.get("litellm_metadata")
    if litellm_metadata is not None:
        return litellm_metadata
    return data.get("metadata")


def recorded_status(data: HookData) -> str | None:
    bucket = metadata_bucket(data)
    if bucket is None:
        return None
    recorded = bucket.get("standard_logging_guardrail_information")
    if recorded is None:
        return None
    status: str | None = None
    for info in recorded:
        status = info["guardrail_status"]
    return status


def applied(data: HookData) -> list[str]:
    bucket = metadata_bucket(data)
    if bucket is None:
        return []
    names = bucket.get("applied_guardrails")
    if names is None:
        return []
    return list(names)


def init_output(call: Input) -> Output:
    try:
        guardrail = build_guardrail(call)
    except (repelloai.RepelloAIGuardrailMissingSecrets, ValueError):
        return {
            "created": False,
            "api_base": None,
            "asset_id": None,
            "key_source": None,
            "event_hooks": event_hooks(),
            "requests": [],
            "applied": [],
            "status": None,
            "caught": None,
            "warned": False,
        }
    return {
        "created": True,
        "api_base": guardrail.api_base,
        "asset_id": guardrail.asset_id,
        "key_source": key_source(call, guardrail.repelloai_api_key),
        "event_hooks": event_hooks(),
        "requests": [],
        "applied": [],
        "status": None,
        "caught": None,
        "warned": False,
    }


def hook_output(call: Input) -> Output:
    guardrail = build_guardrail(call)
    answer = call["answer"]
    if answer is None:
        raise ValueError("the case has no answer")
    script = Script(answer)
    guardrail.async_handler = AsyncHTTPHandler(transport=httpx.MockTransport(script))
    data = call["data"]
    logger = logging.getLogger(LOGGER_NAME)
    counter = WarningCounter()
    level = logger.level
    handlers = logger.handlers[:]
    logger.handlers = [counter]
    logger.setLevel(logging.DEBUG)
    caught: Caught | None = None
    try:
        asyncio.run(
            guardrail.async_pre_call_hook(
                user_api_key_dict=UserAPIKeyAuth(),
                cache=DualCache(),
                data=data,
                call_type="completion",
            )
        )
    except HTTPException as error:
        caught = {"status": error.status_code, "detail": error.detail}
    finally:
        logger.handlers = handlers
        logger.setLevel(level)
    return {
        "created": True,
        "api_base": guardrail.api_base,
        "asset_id": guardrail.asset_id,
        "key_source": key_source(call, guardrail.repelloai_api_key),
        "event_hooks": event_hooks(),
        "requests": script.calls,
        "applied": applied(data),
        "status": recorded_status(data),
        "caught": caught,
        "warned": counter.warned,
    }


def run(input_value: Input) -> Output:
    if input_value["mode"] == "init":
        return init_output(input_value)
    return hook_output(input_value)
