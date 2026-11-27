import asyncio
import sys
from collections.abc import Callable
from types import ModuleType, SimpleNamespace
from typing import ClassVar, Final, Literal, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import AsyncMock

import litellm
from fastapi import HTTPException
from litellm.batches import batch_utils
from litellm.llms.base_llm.files.transformation import BaseFileEndpoints
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.hooks.batch_rate_limiter import (
    BatchFileUsage,
)
from litellm.proxy.hooks.batch_rate_limiter import (
    _PROXY_BatchRateLimiter as BatchRateLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L120
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    _PROXY_MaxParallelRequestsHandler_v3 as ParallelLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L761
)
from litellm.proxy.utils import InternalUsageCache, PrismaClient
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
CALLER: Final = TypeAdapter(UserAPIKeyAuth)
Outcome: TypeAlias = Literal["allow", "deny", "fail", "http"]
Lookup: TypeAlias = Literal["loaded", "failed", "http"]


class HttpView(TypedDict):
    status: int
    detail: NotRequired[JsonValue]


class ContentView(TypedDict):
    contents: NotRequired[str]
    not_bytes: NotRequired[str]
    http: NotRequired[HttpView]


class ManagedView(TypedDict):
    logging: NotRequired[bool]
    hook: NotRequired[Literal["endpoints", "missing", "plain"]]
    router: NotRequired[bool]


class AccessView(TypedDict):
    database: NotRequired[bool]
    team: NotRequired[Lookup]
    team_models: NotRequired[list[str]]
    key: NotRequired[Outcome]
    message: NotRequired[str]


class Input(TypedDict):
    file_id: str
    custom_llm_provider: NotRequired[Literal["openai", "azure", "vertex_ai"]]
    caller: NotRequired[dict[str, JsonValue] | None]
    data: NotRequired[dict[str, JsonValue] | None]
    descriptors: NotRequired[list[RateLimitDescriptor]]
    content: NotRequired[ContentView]
    managed: NotRequired[ManagedView]
    access: NotRequired[AccessView]
    tokens: NotRequired[list[int | None]]


class Effects:
    def __init__(self) -> None:
        self.fetched: list[JsonValue] = []
        self.counted: list[JsonValue] = []


class ScriptedFetch:
    def __init__(self, view: ContentView, effects: Effects) -> None:
        self.view = view
        self.effects = effects

    async def afile_content(
        self, file_id: str, user_api_key_dict: JsonValue, **fetch_kwargs: JsonValue
    ) -> SimpleNamespace:
        recorded: dict[str, JsonValue] = {
            "kind": "provider",
            "file_id": file_id,
            "params": JSON.validate_python(fetch_kwargs),
            "caller": user_api_key_dict is not None,
        }
        self.effects.fetched.append(recorded)
        return self.delivered()

    def delivered(self) -> SimpleNamespace:
        http = self.view.get("http")
        if http is not None:
            raise HTTPException(status_code=http["status"], detail=http.get("detail"))
        not_bytes = self.view.get("not_bytes")
        if not_bytes is not None:
            return SimpleNamespace(content=not_bytes)
        contents = self.view.get("contents")
        return SimpleNamespace(content=b"" if contents is None else contents.encode())


class ManagedHook:
    def __init__(self, fetch: ScriptedFetch) -> None:
        self.fetch = fetch

    async def afile_content(
        self,
        file_id: str,
        litellm_parent_otel_span: JsonValue,
        llm_router: Router,
        **data: JsonValue,
    ) -> SimpleNamespace:
        del litellm_parent_otel_span, llm_router, data
        recorded: dict[str, JsonValue] = {"kind": "managed", "file_id": file_id, "caller": True}
        self.fetch.effects.fetched.append(recorded)
        return self.fetch.delivered()


class FakeUsageCache:
    dual_cache: ClassVar[SimpleNamespace] = SimpleNamespace(redis_cache=None)


class FakeLogging:
    def __init__(self, hook: BaseFileEndpoints | None) -> None:
        self.hook = hook
        self.internal_usage_cache = FakeUsageCache()
        # log_db_metrics schedules this hook after every successful DB read
        self.service_logging_obj = SimpleNamespace(async_service_success_hook=AsyncMock())

    def get_proxy_hook(self, name: str) -> BaseFileEndpoints | None:
        del name
        return self.hook


class ScriptedCache:
    def __init__(self, lookup: Lookup) -> None:
        self.lookup = lookup

    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: JsonValue = None,
        model_type: JsonValue = None,
    ) -> LiteLLM_TeamTable | None:
        del key, parent_otel_span, model_type
        if self.lookup == "failed":
            raise RuntimeError("the team cache could not be read")
        if self.lookup == "http":
            raise HTTPException(status_code=401, detail="passed through")
        return None


class ScriptedCounter:
    def __init__(self, answers: list[int | None] | None, counted: list[JsonValue]) -> None:
        self.answers = [] if answers is None else list(answers)
        self.counted = counted

    def __call__(
        self,
        model: str | None = None,
        messages: list[dict[str, JsonValue]] | None = None,
        text: str | None = None,
    ) -> int:
        if messages is not None:
            roles: list[JsonValue] = [message.get("role") for message in messages]
            self.counted.append({"kind": "messages", "model": model, "roles": roles})
        elif text is not None:
            self.counted.append({"kind": "text", "model": model, "text": text})
        else:
            self.counted.append({"kind": "none", "model": model})
        if not self.answers:
            return 0
        answer = self.answers.pop(0)
        if answer is None:
            raise ValueError("the token counter could not count this row")
        return answer


class TeamRows:
    def __init__(self, models: list[str]) -> None:
        self.row = LiteLLM_TeamTable(team_id="scripted-team", models=list(models))

    async def find_unique(
        self, *, where: dict[str, str], include: dict[str, JsonValue]
    ) -> LiteLLM_TeamTable | None:
        del where, include
        return self.row


class FakeDatabase(PrismaWrapper):
    def __init__(self, teams: TeamRows) -> None:
        self.litellm_teamtable = teams


class FakePrisma(PrismaClient):
    def __init__(self, database: FakeDatabase) -> None:
        self.db = database


class ProxyServer(ModuleType):
    def __init__(self, name: str) -> None:
        super().__init__(name)
        self.llm_router: Router | None = None
        self.prisma_client: PrismaClient | None = None
        self.proxy_logging_obj: FakeLogging | None = None
        self.user_api_key_cache: UserApiKeyCache | ScriptedCache | None = None


def no_model_group(model: str) -> str | None:
    del model
    return None


def build_hook() -> BatchRateLimiter:
    cache = cast(InternalUsageCache, FakeUsageCache())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L129
    limiter = ParallelLimiter(internal_usage_cache=cache, model_group_resolver=no_model_group)
    return BatchRateLimiter(internal_usage_cache=cache, parallel_request_limiter=limiter)


def hook_value(managed: ManagedView, fetch: ScriptedFetch) -> BaseFileEndpoints | None:
    hook = managed.get("hook")
    if hook == "missing":
        return None
    if hook == "plain":
        return cast(BaseFileEndpoints, SimpleNamespace())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L1164
    BaseFileEndpoints.register(ManagedHook)
    return cast(BaseFileEndpoints, ManagedHook(fetch))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/files/transformation.py#L242


def install(call: Input) -> Effects:
    effects = Effects()
    content_view = call.get("content")
    managed_view = call.get("managed")
    access_view = call.get("access")
    content = ContentView() if content_view is None else content_view
    managed = ManagedView() if managed_view is None else managed_view
    access = AccessView() if access_view is None else access_view
    fetch = ScriptedFetch(content, effects)
    logging_off = managed.get("logging") is False
    logging_obj = None if logging_off else FakeLogging(hook_value(managed, fetch))
    router = None if managed.get("router") is False else Router(model_list=[])
    lookup = access.get("team")
    cache = UserApiKeyCache() if lookup is None or lookup == "loaded" else ScriptedCache(lookup)
    proxy = ProxyServer(PROXY_MODULE)
    proxy.llm_router = router
    proxy.proxy_logging_obj = logging_obj
    proxy.user_api_key_cache = cache
    proxy.prisma_client = None
    if access.get("database") is True:
        team_models = access.get("team_models")
        rows = TeamRows([] if team_models is None else team_models)
        proxy.prisma_client = FakePrisma(FakeDatabase(rows))
    sys.modules[PROXY_MODULE] = proxy
    counter = ScriptedCounter(call.get("tokens"), effects.counted)
    batch_utils.token_counter = cast(Callable[..., int], counter)  # type: ignore[attr-defined, explicit-any, misc]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/batch_utils.py#L627
    litellm.afile_content = fetch.afile_content
    return effects


def answer(call: Input, hook: BatchRateLimiter) -> JsonValue:
    effects = install(call)
    caller_view = call.get("caller")
    caller = None if caller_view is None else CALLER.validate_python(caller_view)
    provider = call.get("custom_llm_provider")
    provider_name: Literal["openai", "azure", "vertex_ai"] = "openai"
    if provider is not None:
        provider_name = provider
    usage: BatchFileUsage = asyncio.run(
        hook.count_input_file_usage(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L843
            file_id=call["file_id"],
            custom_llm_provider=provider_name,
            user_api_key_dict=caller,
            data=call.get("data"),
            descriptors=call.get("descriptors"),
        )
    )
    usage_view: dict[str, JsonValue] = {
        "total_tokens": usage.total_tokens,
        "request_count": usage.request_count,
        "output_tokens": usage.output_tokens,
        "per_model_usage": JSON.validate_python(usage.per_model_usage),
    }
    report: dict[str, JsonValue] = {
        "usage": usage_view,
        "fetched": effects.fetched,
        "counted": effects.counted,
    }
    return JSON.validate_python(report)


def run(input_value: Input) -> JsonValue:
    hook = build_hook()
    return answer(input_value, hook)
