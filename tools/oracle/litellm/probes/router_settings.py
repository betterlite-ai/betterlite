import asyncio
import sys
from collections.abc import Generator
from contextlib import contextmanager
from types import ModuleType
from typing import Final, Literal

from litellm.proxy.config_resolvers import FieldSource, SettingsStore
from litellm.proxy.management_endpoints.router_settings_endpoints import (
    get_router_fields,
    get_router_settings,
)
from litellm.router import Router
from litellm.types.litellm_params import RoutingStrategyName
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
DOCUMENT: Final = TypeAdapter(dict[str, JsonValue])
AliasMap = dict[str, str | RouterModelGroupAliasItem]


class RouterSettingsInput(Strict):
    routing_strategy: RoutingStrategyName | None = None
    routing_strategy_args: dict[str, JsonValue] | None = None
    routing_groups: list[dict[str, JsonValue]] | None = None
    num_retries: int | None = None
    timeout: float | None = None
    stream_timeout: float | None = None
    max_fallbacks: int | None = None
    fallbacks: list[JsonValue] | None = None
    context_window_fallbacks: list[JsonValue] | None = None
    content_policy_fallbacks: list[JsonValue] | None = None
    allowed_fails: int | None = None
    cooldown_time: float | None = None
    retry_after: int | None = None
    retry_policy: dict[str, int | None] | None = None
    model_group_alias: dict[str, str] | None = None
    enable_pre_call_checks: bool | None = None
    default_litellm_params: dict[str, JsonValue] | None = None
    set_verbose: bool | None = None
    default_max_parallel_requests: int | None = None
    enable_tag_filtering: bool | None = None
    tag_filtering_match_any: bool | None = None
    disable_cooldowns: bool | None = None


class FieldMetadata(Strict):
    field_name: str
    field_type: str
    field_value: JsonValue
    field_description: str
    field_default: JsonValue
    options: list[str] | None
    ui_field_name: str
    link: str | None


class SettingsOutput(Strict):
    fields: list[FieldMetadata]
    current_values: dict[str, JsonValue]
    routing_strategy_descriptions: dict[str, str]
    routing_group_strategies: list[str]
    source: dict[str, FieldSource]


class FieldsOutput(Strict):
    fields: list[FieldMetadata]
    routing_strategy_descriptions: dict[str, str]
    routing_group_strategies: list[str]


class FakeProxyConfig:
    def __init__(self, store: SettingsStore) -> None:
        self.router_settings: Final = store

    async def get_config(self) -> dict[str, JsonValue]:
        return {"router_settings": dict(self.router_settings.resolved())}


class ProxyServer(ModuleType):
    llm_router: Router
    proxy_config: FakeProxyConfig


@contextmanager
def fake_proxy(router: Router, config: FakeProxyConfig) -> Generator[None]:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.llm_router = router
    proxy_server.proxy_config = config
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    try:
        yield
    finally:
        if saved is None:
            del sys.modules[PROXY_MODULE]
        else:
            sys.modules[PROXY_MODULE] = saved


def settings_document(settings: RouterSettingsInput) -> dict[str, JsonValue]:
    return DOCUMENT.validate_json(settings.model_dump_json(exclude_none=True))


def build_router(settings: RouterSettingsInput) -> Router:
    aliases: AliasMap = AliasMap(settings.model_group_alias or {})
    return Router(
        model_list=[],
        routing_strategy=settings.routing_strategy or "simple-shuffle",
        routing_strategy_args=settings.routing_strategy_args or {},
        routing_groups=settings.routing_groups,
        num_retries=settings.num_retries,
        timeout=settings.timeout,
        stream_timeout=settings.stream_timeout,
        max_fallbacks=settings.max_fallbacks,
        fallbacks=settings.fallbacks,
        context_window_fallbacks=settings.context_window_fallbacks,
        content_policy_fallbacks=settings.content_policy_fallbacks,
        allowed_fails=settings.allowed_fails,
        cooldown_time=settings.cooldown_time,
        # the proxy omits a key the config lacks, and Router's own default for this one is 0 rather than None
        retry_after=settings.retry_after if settings.retry_after is not None else 0,
        retry_policy=settings.retry_policy,
        model_group_alias=aliases,
        enable_pre_call_checks=settings.enable_pre_call_checks or False,
        default_litellm_params=settings.default_litellm_params,
        set_verbose=settings.set_verbose or False,
        default_max_parallel_requests=settings.default_max_parallel_requests,
        enable_tag_filtering=settings.enable_tag_filtering or False,
        tag_filtering_match_any=(
            settings.tag_filtering_match_any if settings.tag_filtering_match_any is not None else True
        ),
        disable_cooldowns=settings.disable_cooldowns,
    )


class RouterSettings(Operation):
    endpoint: Literal["settings", "fields"] = "settings"
    router_settings: RouterSettingsInput = Field(default_factory=RouterSettingsInput)

    @override
    def run(self) -> Strict:
        store = SettingsStore("router_settings")
        store.load_yaml(settings_document(self.router_settings))
        with fake_proxy(build_router(self.router_settings), FakeProxyConfig(store)):
            match self.endpoint:
                case "settings":
                    return SettingsOutput.model_validate_json(asyncio.run(get_router_settings()).model_dump_json())
                case "fields":
                    return FieldsOutput.model_validate_json(asyncio.run(get_router_fields()).model_dump_json())
