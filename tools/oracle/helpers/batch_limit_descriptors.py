import logging
from types import SimpleNamespace
from typing import ClassVar, Final, Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.batch_rate_limiter import (
    _PROXY_BatchRateLimiter as BatchRateLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L120
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitDescriptorRateLimitObject,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    _PROXY_MaxParallelRequestsHandler_v3 as ParallelLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L761
)
from litellm.proxy.utils import InternalUsageCache
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
LOGGER_NAME: Final = "LiteLLM Proxy"
WINDOW_SIZE: Final = 60
Function: TypeAlias = Literal[
    "file_bound_model", "batch_routing_model", "resolve_batch_provider",
    "create_batch_rate_limit_descriptors", "project_has_io_limits",
    "create_project_io_descriptors", "should_skip", "key_requires_access_check",
    "estimate_output_tokens", "has_applicable_limits", "resolve_batch_input_file_fetch_params",
]


class SettingsView(TypedDict):
    # ast-grep-ignore: no-long-names
    disable_batch_input_file_rate_limiting: NotRequired[bool | None]
    # ast-grep-ignore: no-long-names
    skip_batch_input_file_rate_limiting_for_providers: NotRequired[list[JsonValue] | None]
    # ast-grep-ignore: no-long-names
    skip_batch_input_file_rate_limiting_for_models: NotRequired[list[JsonValue] | None]


class RouterScript(TypedDict):
    models: NotRequired[dict[str, dict[str, JsonValue] | None]]
    default: NotRequired[dict[str, JsonValue] | None]


class Call(TypedDict):
    function: Function
    data: dict[str, JsonValue]
    model: str | None
    per_model_usage: dict[str, dict[str, int]]
    has_enqueued_scopes: NotRequired[bool]
    entry: dict[str, JsonValue]
    min_configured_otpm_limit: int | None
    descriptors: list[RateLimitDescriptor]
    file_id: str
    custom_llm_provider: str


class Input(TypedDict):
    settings: SettingsView
    principal: dict[str, JsonValue]
    router: NotRequired[RouterScript | None]
    calls: list[Call]


class _WarningCollector(logging.Handler):
    def __init__(self, events: list[JsonValue]) -> None:
        super().__init__(level=logging.WARNING)
        self._events = events

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self._events.append({"event": "warning", "level": record.levelname.lower()})


class _ScriptedRouter:
    def __init__(self, script: RouterScript, asked: list[JsonValue]) -> None:
        self._script = script
        self._asked = asked

    def get_deployment_credentials_with_provider(  # ast-grep-ignore: no-long-names
        self, model_id: str
    ) -> dict[str, JsonValue] | None:
        self._asked.append(model_id)
        models = self._script.get("models")
        return None if models is None else models.get(model_id)


class _FakeUsageCache:
    dual_cache: ClassVar[SimpleNamespace] = SimpleNamespace(redis_cache=None)


def no_model_group(_model: str) -> str | None:  # ast-grep-ignore: no-one-line-functions
    return None


def descriptor_view(descriptor: RateLimitDescriptor) -> JsonValue:
    # ast-grep-ignore: no-long-names
    rate_limit: RateLimitDescriptorRateLimitObject | None = descriptor.get("rate_limit")
    view: dict[str, JsonValue] = {
        "key": descriptor["key"],
        "value": descriptor["value"],
        "rate_limit": None,
    }
    if rate_limit is not None:
        limits_view: dict[str, JsonValue] = {
            "requests_per_unit": rate_limit.get("requests_per_unit"),
            "tokens_per_unit": rate_limit.get("tokens_per_unit"),
            "max_parallel_requests": rate_limit.get("max_parallel_requests"),
            "window_size": rate_limit.get("window_size"),
        }
        view["rate_limit"] = limits_view
    return JSON.validate_python(view)


def build_hook() -> BatchRateLimiter:
    cache = cast(InternalUsageCache, _FakeUsageCache())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L129
    limiter = ParallelLimiter(internal_usage_cache=cache, model_group_resolver=no_model_group)
    limiter.window_size = WINDOW_SIZE
    return BatchRateLimiter(internal_usage_cache=cache, parallel_request_limiter=limiter)


def settings_dict(settings: SettingsView) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {}
    disable = settings.get("disable_batch_input_file_rate_limiting")
    if disable is not None:
        values["disable_batch_input_file_rate_limiting"] = disable
    providers = settings.get("skip_batch_input_file_rate_limiting_for_providers")
    if providers is not None:
        values["skip_batch_input_file_rate_limiting_for_providers"] = providers
    models = settings.get("skip_batch_input_file_rate_limiting_for_models")
    if models is not None:
        values["skip_batch_input_file_rate_limiting_for_models"] = models
    return values


def answer(call: Call, hook: BatchRateLimiter, auth: UserAPIKeyAuth) -> JsonValue:
    match call["function"]:
        case "file_bound_model":
            return {"function": "file_bound_model", "model": hook._get_file_bound_batch_model(call["data"])}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L151
        case "batch_routing_model":
            return {"function": "batch_routing_model", "model": hook._get_batch_routing_model(call["data"])}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L182
        case "resolve_batch_provider":
            return {"function": "resolve_batch_provider", "provider": hook._resolve_batch_provider(call["model"])}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L201
        case "create_batch_rate_limit_descriptors":
            descriptors = hook._create_batch_rate_limit_descriptors(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L231
                user_api_key_dict=auth, data=call["data"]
            )
            return {
                "function": "create_batch_rate_limit_descriptors",
                "descriptors": [descriptor_view(descriptor) for descriptor in descriptors],
            }
        case "project_has_io_limits":
            return {"function": "project_has_io_limits", "has_limits": BatchRateLimiter._project_has_any_io_token_limits(auth)}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L288
        case _:
            return answer_usage(call, hook, auth)


def answer_usage(call: Call, hook: BatchRateLimiter, auth: UserAPIKeyAuth) -> JsonValue:
    if call["function"] == "create_project_io_descriptors":
        descriptors, increments = hook._create_project_io_descriptors_for_models(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L304
            user_api_key_dict=auth, per_model_usage=call["per_model_usage"]
        )
        return {
            "function": "create_project_io_descriptors",
            "descriptors": [descriptor_view(descriptor) for descriptor in descriptors],
            "increments": [
                {"requests": increment["requests"], "tokens": increment["tokens"]}
                for increment in increments
            ],
        }
    if call["function"] == "should_skip":
        skipped, skip_descriptors = hook._should_skip_batch_input_file_processing(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L338
            call["data"], auth, has_enqueued_scopes=bool(call.get("has_enqueued_scopes"))
        )
        return {
            "function": "should_skip",
            "skip": skipped,
            "descriptors": None
            if skip_descriptors is None
            else [descriptor_view(descriptor) for descriptor in skip_descriptors],
        }
    if call["function"] == "key_requires_access_check":
        return {"function": "key_requires_access_check", "required": BatchRateLimiter._key_requires_batch_model_access_check(auth)}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L424
    if call["function"] == "estimate_output_tokens":
        return {"function": "estimate_output_tokens", "tokens": hook._estimate_entry_output_tokens(call["entry"], call["min_configured_otpm_limit"])}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L439
    if call["function"] == "has_applicable_limits":
        return {"function": "has_applicable_limits", "applicable": BatchRateLimiter._has_applicable_batch_rate_limits(call["descriptors"])}  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L492
    resolved = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L563
        "tuple[str, dict[str, JsonValue]]",
        hook._resolve_batch_input_file_fetch_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L505
            call["file_id"], call["custom_llm_provider"], call["data"]
        ),
    )
    file_id, params = resolved
    return {
        "function": "resolve_batch_input_file_fetch_params",
        "file_id": file_id,
        "params": params,
    }


def run(input_value: Input) -> JsonValue:
    events: list[JsonValue] = []
    asked: list[JsonValue] = []
    router: _ScriptedRouter | None = None
    if "router" in input_value:
        script = input_value["router"]
        router = _ScriptedRouter(RouterScript() if script is None else script, asked)
    auth = TypeAdapter(UserAPIKeyAuth).validate_python(input_value["principal"])
    hook = build_hook()
    logger = logging.getLogger(LOGGER_NAME)
    logger.setLevel(logging.WARNING)
    collector = _WarningCollector(events)
    logger.addHandler(collector)
    proxy_server.llm_router = None if router is None else cast(Router, router)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2584
    proxy_server.general_settings = settings_dict(input_value["settings"])
    try:
        answers = [answer(call, hook, auth) for call in input_value["calls"]]
    finally:
        logger.removeHandler(collector)
    report: dict[str, JsonValue] = {"answers": answers, "events": events, "asked": asked}
    return JSON.validate_python(report)
