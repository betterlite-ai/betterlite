import asyncio
import sys
from collections.abc import Sequence
from types import ModuleType
from typing import Required, TypedDict

from fastapi import HTTPException
from litellm.llms.litellm_proxy.skills import skill_search
from litellm.llms.litellm_proxy.skills.skill_search import (
    DEFAULT_SKILL_SEARCH_TOP_K,
    HostedSkillSearchOutcome,
    SkillSearchEmbeddingFailed,
    SkillSearchHits,
    SkillSearchIndex,
    SkillSearchNotConfigured,
    SkillSearchUnsupportedProvider,
    search_hosted_skills,
    search_skills,
    skill_search_result,
    skill_search_text,
)
from litellm.models.skills import LiteLLM_SkillsTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.semantic_text_index import Embedder, Vector
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import ProxyLogging
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE = "litellm.proxy.proxy_server"


class PrincipalFields(TypedDict, total=False):
    user_id: str
    team_id: str
    org_id: str
    api_key: str
    token: str


class SkillFields(TypedDict, total=False):
    skill_id: Required[str]
    display_title: str | None
    description: str | None
    instructions: str | None
    created_by: str | None


class Input(TypedDict, total=False):
    function: Required[str]
    query: str
    top_k: int | None
    embedding_model: str | None
    custom_llm_provider: str | None
    router: bool
    principal: PrincipalFields
    skill: SkillFields
    skills: list[SkillFields]
    rows: list[SkillFields]
    vectors: dict[str, list[float]]
    fault: str


class ResultFields(TypedDict):
    skill_id: str
    display_title: str | None
    description: str | None
    score: float


class Order(TypedDict):
    created_at: str


ScopeFilter = TypedDict("ScopeFilter", {"in": list[str]})


class Where(TypedDict):
    created_by: ScopeFilter


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
RESULT: TypeAdapter[ResultFields] = TypeAdapter(ResultFields)


def build_skill(fields: SkillFields) -> LiteLLM_SkillsTable:
    return LiteLLM_SkillsTable(
        skill_id=fields["skill_id"],
        display_title=fields.get("display_title"),
        description=fields.get("description"),
        instructions=fields.get("instructions"),
        created_by=fields.get("created_by"),
    )


def skill_rows_of(input_value: Input) -> list[LiteLLM_SkillsTable]:
    fields = input_value.get("skills")
    if fields is None:
        return []
    return [build_skill(skill) for skill in fields]


def row_fields_of(input_value: Input) -> list[LiteLLM_SkillsTable]:
    fields = input_value.get("rows")
    if fields is None:
        return []
    return [build_skill(row) for row in fields]


def vectors_of(input_value: Input) -> dict[str, list[float]]:
    vectors = input_value.get("vectors")
    if vectors is None:
        return {}
    return vectors


def top_k_of(input_value: Input) -> int:
    top_k = input_value.get("top_k")
    if top_k is None:
        return DEFAULT_SKILL_SEARCH_TOP_K
    return top_k


def embedding_model_of(input_value: Input) -> str:
    model = input_value.get("embedding_model")
    if model is None:
        raise ValueError("embedding_model is required")
    return model


def principal_of(fields: PrincipalFields) -> UserAPIKeyAuth:
    # The handler receives resolved auth fields; construction would replace token with the api key.
    principal = UserAPIKeyAuth(
        user_id=fields.get("user_id"),
        team_id=fields.get("team_id"),
        org_id=fields.get("org_id"),
    )
    principal.api_key = fields.get("api_key")
    principal.token = fields.get("token")
    return principal


class ScriptedEmbedder:
    def __init__(self, request: Input, effects: list[JsonValue]) -> None:
        self.request = request
        self.effects = effects

    async def __call__(self, texts: Sequence[str]) -> Sequence[Vector]:
        self.effects.append(JSON.validate_python({"kind": "embed", "texts": list(texts)}))
        fault = self.request.get("fault")
        if fault == "value":
            raise ValueError("boom")
        if fault == "http":
            raise HTTPException(status_code=429, detail="slow down")
        vectors = vectors_of(self.request)
        return [tuple(vectors[text]) for text in texts]


class ScriptedRouterEmbedder:
    def __init__(
        self,
        request: Input,
        effects: list[JsonValue],
        router: Router | None,
        principal: UserAPIKeyAuth,
        proxy_logging: ProxyLogging,
    ) -> None:
        self.request = request
        self.effects = effects
        self.router = router
        self.principal = principal
        self.proxy_logging = proxy_logging

    def __call__(
        self,
        router: Router,
        embedding_model: str,
        user_api_key_dict: UserAPIKeyAuth,
        proxy_logging_obj: ProxyLogging,
    ) -> Embedder:
        if router is not self.router:
            raise RuntimeError("skill search resolved the embedder from another router")
        if embedding_model != self.request.get("embedding_model"):
            raise RuntimeError("skill search resolved the embedder for another model")
        if user_api_key_dict is not self.principal:
            raise RuntimeError("skill search resolved the embedder for another caller")
        if proxy_logging_obj is not self.proxy_logging:
            raise RuntimeError("skill search resolved the embedder with another logging object")
        return ScriptedEmbedder(self.request, self.effects)


class FakeTable:
    def __init__(self, request: Input, effects: list[JsonValue]) -> None:
        self.rows = row_fields_of(request)
        self.effects = effects

    async def find_many(
        self, take: int, skip: int, order: Order, where: Where | None
    ) -> list[LiteLLM_SkillsTable]:
        effect = {"kind": "find_many", "take": take, "skip": skip, "order": order, "where": where}
        self.effects.append(JSON.validate_python(effect))
        if where is None:
            visible = self.rows
        else:
            allowed = where["created_by"]["in"]
            visible = [row for row in self.rows if row.created_by in allowed]
        return visible[skip:][:take]


class FakeClient:
    def __init__(self, table: FakeTable) -> None:
        self.db: FakeClient = self
        self.litellm_skillstable = table


class FakeModule(ModuleType):
    prisma_client: FakeClient | None = None


def failure_json(outcome: str, reason: str, effects: list[JsonValue]) -> JsonValue:
    value = {"outcome": outcome, "reason": reason, "hits": [], "effects": effects}
    return JSON.validate_python(value)


def outcome_json(outcome: HostedSkillSearchOutcome, effects: list[JsonValue]) -> JsonValue:
    match outcome:
        case SkillSearchHits(hits=hits):
            results = [
                RESULT.validate_json(skill_search_result(hit).model_dump_json()) for hit in hits
            ]
            value = {"outcome": "hits", "reason": None, "hits": results, "effects": effects}
            return JSON.validate_python(value)
        case SkillSearchNotConfigured(reason=reason):
            return failure_json("not_configured", reason, effects)
        case SkillSearchEmbeddingFailed(reason=reason):
            return failure_json("failed", reason, effects)
        case SkillSearchUnsupportedProvider(reason=reason):
            return failure_json("unsupported_provider", reason, effects)


def text_value(input_value: Input) -> JsonValue:
    skill = build_skill(input_value["skill"])
    return JSON.validate_python({"text": skill_search_text(skill)})


async def index_value(input_value: Input) -> JsonValue:
    effects: list[JsonValue] = []
    embedder = ScriptedEmbedder(input_value, effects)
    outcome = await SkillSearchIndex().search(
        input_value["query"],
        skill_rows_of(input_value),
        top_k_of(input_value),
        embedder,
        embedding_model_of(input_value),
    )
    return outcome_json(outcome, effects)


async def search_value(input_value: Input) -> JsonValue:
    effects: list[JsonValue] = []
    router = Router(model_list=[]) if input_value.get("router") is True else None
    principal = UserAPIKeyAuth()
    proxy_logging = ProxyLogging(user_api_key_cache=UserApiKeyCache())
    skill_search.router_embedder = ScriptedRouterEmbedder(
        input_value, effects, router, principal, proxy_logging
    )
    outcome = await search_skills(
        query=input_value["query"],
        skills=skill_rows_of(input_value),
        top_k=top_k_of(input_value),
        router=router,
        embedding_model=input_value.get("embedding_model"),
        index=SkillSearchIndex(),
        user_api_key_dict=principal,
        proxy_logging_obj=proxy_logging,
    )
    return outcome_json(outcome, effects)


async def hosted_value(input_value: Input) -> JsonValue:
    effects: list[JsonValue] = []
    proxy = FakeModule(PROXY_MODULE)
    proxy.prisma_client = FakeClient(FakeTable(input_value, effects))
    sys.modules[PROXY_MODULE] = proxy
    principal_fields = input_value.get("principal")
    principal = UserAPIKeyAuth() if principal_fields is None else principal_of(principal_fields)
    router = Router(model_list=[]) if input_value.get("router") is True else None
    proxy_logging = ProxyLogging(user_api_key_cache=UserApiKeyCache())
    skill_search.router_embedder = ScriptedRouterEmbedder(
        input_value, effects, router, principal, proxy_logging
    )
    outcome = await search_hosted_skills(
        custom_llm_provider=input_value.get("custom_llm_provider"),
        query=input_value["query"],
        top_k=top_k_of(input_value),
        router=router,
        embedding_model=input_value.get("embedding_model"),
        index=SkillSearchIndex(),
        user_api_key_dict=principal,
        proxy_logging_obj=proxy_logging,
    )
    return outcome_json(outcome, effects)


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "text":
        return text_value(input_value)
    if function in {"rank", "failures"}:
        return asyncio.run(index_value(input_value))
    if function == "search":
        return asyncio.run(search_value(input_value))
    if function == "hosted":
        return asyncio.run(hosted_value(input_value))
    raise ValueError(f"unknown function: {function}")
