import asyncio
import io
from contextlib import redirect_stdout
from typing import Literal, TypedDict, cast

from litellm.caching.dual_cache import DualCache
from litellm.exceptions import ModifyResponseException
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.semantic_guard.semantic_guard import SemanticGuardrail
from litellm.router import Router
from litellm.types.utils import Embedding, EmbeddingResponse
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException


class RouteDefinition(TypedDict):
    route_name: str
    utterances: list[str]


class Settings(TypedDict):
    guardrail_name: str
    embedding_model: str
    similarity_threshold: float
    custom_routes: list[RouteDefinition]
    on_flagged_action: str
    default_on: bool


class ResponseChoice(TypedDict):
    content: JsonValue


class ResponseBody(TypedDict):
    choices: list[ResponseChoice]


class Call(TypedDict):
    request: dict[str, JsonValue]
    call_type: str
    matches: bool
    response: ResponseBody | None


class Input(TypedDict):
    settings: Settings
    calls: list[Call]


class Detail(TypedDict):
    error: str
    route: str
    similarity_score: float | None
    type: str


class DetectionInfo(TypedDict):
    route_name: str
    similarity_score: float | None
    guardrail: str | None


class CallReport(TypedDict):
    scored: list[str]
    kind: Literal["passed", "blocked", "passthrough"]
    status: int | None
    detail: Detail | None
    message: str | None
    model: str | None
    guardrail_name: str | None
    request_data: dict[str, JsonValue] | None
    detection_info: DetectionInfo | None
    returned: ResponseBody | None


class Report(TypedDict):
    provider: str
    embedding_model: str
    similarity_threshold: float
    on_flagged_action: str
    route_count: int
    default_on: bool
    supported_event_hooks: list[str]
    calls: list[CallReport]


DETAIL_ADAPTER: TypeAdapter[Detail] = TypeAdapter(Detail)
DETECTION_ADAPTER: TypeAdapter[DetectionInfo] = TypeAdapter(DetectionInfo)
REQUEST_DATA_ADAPTER: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class _Message:
    def __init__(self, content: JsonValue) -> None:
        self.content = content


class _Choice:
    def __init__(self, content: JsonValue) -> None:
        self.message = _Message(content)


class _Response:
    def __init__(self, body: ResponseBody) -> None:
        self.choices = [_Choice(choice["content"]) for choice in body["choices"]]


# a case cannot run the embedding service, so it scripts which route the router answers
class _EmbeddingRouter(Router):
    def __init__(self) -> None:
        super().__init__()
        self.matches = False
        self.active = False
        self.scored: list[str] = []

    def embedding(
        self,
        model: str,
        input: str | list[str],  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L5863
        is_async: bool | None = False,  # noqa: ARG002, FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L5863
        **_kwargs: JsonValue,
    ) -> EmbeddingResponse:
        texts: list[str] = []
        match input:
            case str():
                texts = [input]
            case list():
                texts = input
        if self.active:
            self.scored.extend(texts)
        vector = [1.0, 0.0] if not self.active or self.matches else [0.0, 1.0]
        data = [
            Embedding(embedding=vector, index=index, object="embedding")
            for index, _ in enumerate(texts)
        ]
        return EmbeddingResponse(model=model, data=data)


def build_guardrail(settings: Settings) -> tuple[SemanticGuardrail, _EmbeddingRouter]:
    router = _EmbeddingRouter()
    guardrail = SemanticGuardrail(
        guardrail_name=settings["guardrail_name"],
        llm_router=router,
        embedding_model=settings["embedding_model"],
        similarity_threshold=settings["similarity_threshold"],
        custom_routes=cast("list[dict[str, object]]", settings["custom_routes"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/semantic_guard/semantic_guard.py#L54
        on_flagged_action=settings["on_flagged_action"],
        default_on=settings["default_on"],
    )
    router.active = True
    return guardrail, router


def passed_report(router: _EmbeddingRouter, returned: ResponseBody | None) -> CallReport:
    return {
        "scored": list(router.scored),
        "kind": "passed",
        "status": None,
        "detail": None,
        "message": None,
        "model": None,
        "guardrail_name": None,
        "request_data": None,
        "detection_info": None,
        "returned": returned,
    }


def blocked_report(router: _EmbeddingRouter, error: HTTPException) -> CallReport:
    return {
        "scored": list(router.scored),
        "kind": "blocked",
        "status": error.status_code,
        "detail": DETAIL_ADAPTER.validate_python(error.detail),
        "message": None,
        "model": None,
        "guardrail_name": None,
        "request_data": None,
        "detection_info": None,
        "returned": None,
    }


def passthrough_report(router: _EmbeddingRouter, error: ModifyResponseException) -> CallReport:
    return {
        "scored": list(router.scored),
        "kind": "passthrough",
        "status": None,
        "detail": None,
        "message": error.message,
        "model": error.model,
        "guardrail_name": error.guardrail_name,
        "request_data": REQUEST_DATA_ADAPTER.validate_python(error.request_data),
        "detection_info": DETECTION_ADAPTER.validate_python(error.detection_info),
        "returned": None,
    }


def run_call(guardrail: SemanticGuardrail, router: _EmbeddingRouter, call: Call) -> CallReport:
    router.matches = call["matches"]
    router.scored.clear()
    body = call["response"]
    try:
        if body is None:
            asyncio.run(guardrail.async_pre_call_hook(UserAPIKeyAuth(), DualCache(), call["request"], call["call_type"]))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/semantic_guard/semantic_guard.py#L108
        else:
            asyncio.run(guardrail.async_post_call_success_hook(call["request"], UserAPIKeyAuth(), _Response(body)))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportAny, reportArgumentType, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/semantic_guard/semantic_guard.py#L139
    except HTTPException as error:
        return blocked_report(router, error)
    except ModifyResponseException as error:
        return passthrough_report(router, error)
    return passed_report(router, body)


def run(input_value: Input) -> Report:
    # the guardrail and the router log at INFO to sys.stdout, which carries the oracle's JSON line
    with redirect_stdout(io.StringIO()):
        guardrail, router = build_guardrail(input_value["settings"])
        hooks = guardrail.supported_event_hooks
        if hooks is None:
            raise RuntimeError("SemanticGuardrail did not install its supported event hooks")
        calls = [run_call(guardrail, router, call) for call in input_value["calls"]]
        return {
            "provider": guardrail.guardrail_provider,
            "embedding_model": guardrail.embedding_model,
            "similarity_threshold": guardrail.similarity_threshold,
            "on_flagged_action": guardrail.on_flagged_action,
            "route_count": guardrail.route_count,
            "default_on": guardrail.default_on,
            "supported_event_hooks": [hook.value for hook in hooks],
            "calls": calls,
        }
