import asyncio
from collections.abc import Iterator
from contextlib import contextmanager
from functools import partial
from typing import Literal, TypeAlias, TypedDict

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
Request: TypeAlias = dict[str, object]  # ast-grep-ignore: no-loose-types
FIXED_TOKEN = "fixed-token"
REQUEST = TypeAdapter(Request)
WIRE = TypeAdapter(dict[str, Json])


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


class MarkPreCallInput(Settings):
    function: Literal["mark_pre_call"]
    data: dict[str, Json]


class AlreadyRanInput(Settings):
    function: Literal["already_ran"]
    data: dict[str, Json]


class StampedInput(Settings):
    function: Literal["stamped_key_fields"]
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
    | MarkPreCallInput
    | AlreadyRanInput
    | StampedInput
    | DeploymentPreInput
    | DeploymentPostInput
)


class Executed(TypedDict):
    _pre_call_executed_guardrails: list[str]


class Marked(TypedDict):
    metadata: Executed


class Probe(CustomGuardrail):
    def __init__(self, settings: Settings) -> None:
        super().__init__(
            guardrail_name=settings["guardrail_name"],
            event_hook=TypeAdapter(EventHook).validate_python(settings["event_hook"]),
            default_on=settings["default_on"],
            only_scan_new_messages=settings["only_scan_new_messages"],
            mask_request_content=settings["mask_request_content"],
            mask_response_content=settings["mask_response_content"],
        )
        self.keys: list[UserAPIKeyAuth] = []

    @override
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: dict[str, Json],
        call_type: str,
    ) -> dict[str, Json]:
        del cache, data, call_type
        self.keys.append(user_api_key_dict)
        return {"messages": [{"role": "user", "content": "hooked"}]}

    @override
    async def async_post_call_success_hook(
        self,
        data: dict[str, Json],
        user_api_key_dict: UserAPIKeyAuth,
        response: LLMResponseTypes,
    ) -> ModelResponse:
        del data, user_api_key_dict, response
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
        await super().async_set_cache(key, value, local_only, **kwargs)


def pre_call_token() -> str:
    # Nothing public names the process token, but a marked request carries it
    scratch: Request = {}
    CustomGuardrail(guardrail_name="probe").mark_pre_call_hook_ran(scratch)
    marked = TypeAdapter(Marked).validate_python(scratch)
    return marked["metadata"]["_pre_call_executed_guardrails"][0].removesuffix(":probe")


def swap(value: Request, old: str, new: str) -> Request:
    return REQUEST.validate_json(REQUEST.dump_json(value).decode().replace(old, new))


@contextmanager
def suppressing(names: list[str]) -> Iterator[None]:
    original = auto_router_compression.suppressed_compression_guardrails
    auto_router_compression.suppressed_compression_guardrails = partial(frozenset, names)
    try:
        yield
    finally:
        auto_router_compression.suppressed_compression_guardrails = original


async def seeded(name: str | None, texts: list[str], data: dict[str, Json]) -> StoredCache:
    cache = StoredCache()
    seeding = Settings(
        guardrail_name=name,
        event_hook=None,
        default_on=False,
        only_scan_new_messages=True,
        mask_request_content=False,
        mask_response_content=False,
    )
    await Probe(seeding).mark_texts_scanned(texts, REQUEST.validate_python(data), cache)
    return cache


async def scan_filter(args: ScanFilterInput) -> Json:
    cache = await seeded(args["guardrail_name"], args["cached"], args["data"])
    return await Probe(args).filter_new_texts_for_session(
        args["texts"], REQUEST.validate_python(args["data"]), cache
    )


async def scan_mark(args: ScanMarkInput) -> Json:
    cache = await seeded(args["guardrail_name"], args["existing"], args["data"])
    data = REQUEST.validate_python(args["data"])
    await Probe(args).mark_texts_scanned(args["texts"], data, cache)
    return cache.last


def mark_pre_call(args: MarkPreCallInput) -> Json:
    token = pre_call_token()
    scratch = swap(REQUEST.validate_python(args["data"]), FIXED_TOKEN, token)
    Probe(args).mark_pre_call_hook_ran(scratch)
    return WIRE.validate_python(swap(scratch, token, FIXED_TOKEN))


async def already_ran(args: AlreadyRanInput) -> Json:
    token = pre_call_token()
    kwargs = swap({"guardrails": [], **args["data"]}, FIXED_TOKEN, token)
    probe = Probe(args)
    await probe.async_pre_call_deployment_hook(kwargs, CallTypes.completion)
    return not probe.keys


async def stamped_key_fields(args: StampedInput) -> Json:
    probe = Probe(args)
    kwargs = REQUEST.validate_python({"guardrails": [], **args["data"]})
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
    kwargs = swap(REQUEST.validate_python(args["kwargs"]), FIXED_TOKEN, token)
    call_type = None if args["call_type"] is None else CallTypes(args["call_type"])
    try:
        with suppressing(args["suppressed"]):
            await Probe(args).async_pre_call_deployment_hook(kwargs, call_type)
    except ImportError:
        return "rejected"
    return WIRE.validate_python(swap(kwargs, token, FIXED_TOKEN))


async def post_hook(args: DeploymentPostInput) -> Json:
    try:
        with suppressing(args["suppressed"]):
            result = await Probe(args).async_post_call_success_deployment_hook(
                REQUEST.validate_python(args["request_data"]), ModelResponse(), None
            )
    except ImportError:
        return "rejected"
    return result is not None


def run(input_value: Input) -> dict[str, Json]:
    match input_value["function"]:
        case "scan_filter":
            return {"result": asyncio.run(scan_filter(input_value))}
        case "scan_mark":
            return {"result": asyncio.run(scan_mark(input_value))}
        case "mark_pre_call":
            return {"result": mark_pre_call(input_value)}
        case "already_ran":
            return {"result": asyncio.run(already_ran(input_value))}
        case "stamped_key_fields":
            return {"result": asyncio.run(stamped_key_fields(input_value))}
        case "deployment_pre":
            return {"result": asyncio.run(pre_hook(input_value))}
        case "deployment_post":
            return {"result": asyncio.run(post_hook(input_value))}
