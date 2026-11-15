import asyncio
from collections.abc import Generator
from contextlib import contextmanager
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

from litellm.caching.dual_cache import DualCache
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails import auto_router_compression
from litellm.types.guardrails import GuardrailEventHooks, Mode
from litellm.types.utils import CallTypes, LLMResponseTypes, ModelResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override

EventHook: TypeAlias = GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None
# The request dict LiteLLM declares as dict[str, object]: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L480
Request: TypeAlias = dict[str, object]
# LiteLLM returns LLMResponseTypes | None and the helper only asks whether it is None: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L905-L910
Response: TypeAlias = object
FIXED_MARKER: Final = "fixed-token"
REQUEST: Final[TypeAdapter[Request]] = TypeAdapter(Request)
WIRE: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])
JSON: Final[TypeAdapter[Json]] = TypeAdapter(Json)


class Settings(TypedDict):
    guardrail_name: str | None
    event_hook: Json
    default_on: bool
    only_scan_new_messages: bool
    mask_request_content: bool
    mask_response_content: bool


class ScanFilterInput(Settings):
    function: Literal["scan_filter"]
    texts: list[str] | None
    cached: list[str]
    data: dict[str, Json]


class ScanMarkInput(Settings):
    function: Literal["scan_mark"]
    texts: list[str] | None
    existing: list[str]
    data: dict[str, Json]


class DataInput(Settings):
    function: Literal["mark_pre_call", "already_ran", "stamped_key_fields"]
    data: dict[str, Json]


class DeploymentPreInput(Settings):
    function: Literal["deployment_pre"]
    kwargs: dict[str, Json]
    call_type: str | None
    suppressed: list[str]


class DeploymentPostInput(Settings):
    function: Literal["deployment_post"]
    request_data: dict[str, Json]
    suppressed: list[str]


Input: TypeAlias = (
    ScanFilterInput
    | ScanMarkInput
    | DataInput
    | DeploymentPreInput
    | DeploymentPostInput
)


class Executed(TypedDict):
    _pre_call_executed_guardrails: list[str]


class Marked(TypedDict):
    metadata: Executed


class Hooked(Protocol):
    keys: list[UserAPIKeyAuth]

    def mark_pre_call_hook_ran(self, data: Request) -> None: ...

    async def async_pre_call_deployment_hook(
        self, kwargs: Request, call_type: CallTypes | None
    ) -> Request | None: ...

    async def async_post_call_success_deployment_hook(  # ast-grep-ignore: no-long-names
        self, request_data: Request, response: ModelResponse, call_type: CallTypes | None
    ) -> Response | None: ...

    async def filter_new_texts_for_session(
        self, texts: list[str] | None, request_data: Request, cache: DualCache
    ) -> list[str] | None: ...

    async def mark_texts_scanned(
        self, texts: list[str] | None, request_data: Request, cache: DualCache
    ) -> None: ...


# a custom metaclass skips mypy's __init_subclass__ simulation, which reports Any from **kwargs
class ProbeMeta(type):
    pass


class Probe(CustomGuardrail, metaclass=ProbeMeta):
    keys: list[UserAPIKeyAuth]

    @override
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: dict[str, Json],
        call_type: str,
    ) -> dict[str, Json]:
        self.keys.append(user_api_key_dict)
        return {"messages": [{"role": "user", "content": "hooked"}]}

    @override
    async def async_post_call_success_hook(
        self,
        data: dict[str, Json],
        user_api_key_dict: UserAPIKeyAuth,
        response: LLMResponseTypes,
    ) -> ModelResponse:
        return ModelResponse()


class StoredCache(DualCache):
    def __init__(self) -> None:
        super().__init__()
        self.last: list[str] = []

    @override
    async def async_set_cache(
        self, key: str, value: list[str], local_only: bool = False, **kwargs: Json
    ) -> None:
        self.last = value
        await super().async_set_cache(key=key, value=value, local_only=local_only, **kwargs)


def build_probe(settings: Settings) -> Hooked:
    event_hook = TypeAdapter(EventHook).validate_python(settings["event_hook"])
    probe = Probe(
        guardrail_name=settings["guardrail_name"],
        event_hook=event_hook,
        default_on=settings["default_on"],
        only_scan_new_messages=settings["only_scan_new_messages"],
        mask_request_content=settings["mask_request_content"],
        mask_response_content=settings["mask_response_content"],
    )
    probe.keys = []
    return probe


def pre_call_token() -> str:
    # Nothing public names the process token, but a marked request carries it
    scratch: Request = {}
    CustomGuardrail(guardrail_name="probe").mark_pre_call_hook_ran(scratch)
    marked = TypeAdapter(Marked).validate_python(scratch)
    return marked["metadata"]["_pre_call_executed_guardrails"][0].removesuffix(":probe")


@contextmanager
def suppressing(names: list[str]) -> Generator[None]:
    original = auto_router_compression.suppressed_compression_guardrails
    auto_router_compression.suppressed_compression_guardrails = partial(frozenset, names)
    try:
        yield
    finally:
        auto_router_compression.suppressed_compression_guardrails = original


async def seeded(name: str | None, texts: list[str], data: dict[str, Json]) -> StoredCache:
    cache = StoredCache()
    await Probe(guardrail_name=name, only_scan_new_messages=True).mark_texts_scanned(
        texts, REQUEST.validate_python(data), cache
    )
    return cache


async def scan_filter(args: ScanFilterInput) -> Json:
    cache = await seeded(args["guardrail_name"], args["cached"], args["data"])
    request = REQUEST.validate_python(args["data"])
    kept = await build_probe(args).filter_new_texts_for_session(args["texts"], request, cache)
    return JSON.validate_python(kept)


async def scan_mark(args: ScanMarkInput) -> Json:
    cache = await seeded(args["guardrail_name"], args["existing"], args["data"])
    request = REQUEST.validate_python(args["data"])
    await build_probe(args).mark_texts_scanned(args["texts"], request, cache)
    return JSON.validate_python(cache.last)


def mark_pre_call(args: DataInput) -> Json:
    token = pre_call_token()
    scratch = REQUEST.validate_json(
        WIRE.dump_json(args["data"]).decode().replace(FIXED_MARKER, token)
    )
    build_probe(args).mark_pre_call_hook_ran(scratch)
    return WIRE.validate_json(REQUEST.dump_json(scratch).decode().replace(token, FIXED_MARKER))


async def already_ran(args: DataInput) -> Json:
    token = pre_call_token()
    data = WIRE.dump_json({"guardrails": [], **args["data"]}).decode()
    kwargs = REQUEST.validate_json(data.replace(FIXED_MARKER, token))
    probe = build_probe(args)
    await probe.async_pre_call_deployment_hook(kwargs, CallTypes.completion)
    return not probe.keys


async def stamped_key_fields(args: DataInput) -> Json:
    probe = build_probe(args)
    kwargs = REQUEST.validate_json(WIRE.dump_json({"guardrails": [], **args["data"]}).decode())
    await probe.async_pre_call_deployment_hook(kwargs, CallTypes.completion)
    [key] = probe.keys
    return {
        "user_id": key.user_id,
        "team_id": key.team_id,
        "end_user_id": key.end_user_id,
        "api_key": key.api_key,
        "request_route": key.request_route,
    }


async def pre_hook(args: DeploymentPreInput) -> Json:
    token = pre_call_token()
    kwargs = REQUEST.validate_json(
        WIRE.dump_json(args["kwargs"]).decode().replace(FIXED_MARKER, token)
    )
    call_type = None if args["call_type"] is None else CallTypes(args["call_type"])
    try:
        with suppressing(args["suppressed"]):
            await build_probe(args).async_pre_call_deployment_hook(kwargs, call_type)
    except ImportError:
        return "rejected"
    return WIRE.validate_json(REQUEST.dump_json(kwargs).decode().replace(token, FIXED_MARKER))


async def post_hook(args: DeploymentPostInput) -> Json:
    try:
        with suppressing(args["suppressed"]):
            result = await build_probe(args).async_post_call_success_deployment_hook(
                REQUEST.validate_python(args["request_data"]), ModelResponse(), None
            )
    except ImportError:
        return "rejected"
    return result is not None


def run(input_value: Input) -> dict[str, Json]:
    result: Json = None
    match input_value["function"]:
        case "scan_filter":
            result = asyncio.run(scan_filter(input_value))
        case "scan_mark":
            result = asyncio.run(scan_mark(input_value))
        case "mark_pre_call":
            result = mark_pre_call(input_value)
        case "already_ran":
            result = asyncio.run(already_ran(input_value))
        case "stamped_key_fields":
            result = asyncio.run(stamped_key_fields(input_value))
        case "deployment_pre":
            result = asyncio.run(pre_hook(input_value))
        case "deployment_post":
            result = asyncio.run(post_hook(input_value))
    return {"result": result}
