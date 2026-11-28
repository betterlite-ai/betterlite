from collections.abc import Callable
from datetime import datetime
from typing import Final, Protocol, TypedDict, Unpack, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai import vertex_ai_partner_models as partner_module
from litellm.llms.vertex_ai.gemini_embeddings import (
    batch_embed_content_transformation as embed_module,
)
from litellm.llms.vertex_ai.image_generation import (
    image_generation_handler as image_module,
)
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    vertex_passthrough_logging_handler as handler_module,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter

START: Final = datetime.fromisoformat("2026-01-01T00:00:00+00:00")
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class Input(TypedDict):
    url_route: str


class HandlerCall(TypedDict):
    httpx_response: httpx.Response
    logging_obj: Logging
    url_route: str
    result: str
    start_time: datetime
    end_time: datetime
    cache_hit: bool


@runtime_checkable
class VertexHandler(Protocol):
    batch_prediction_jobs_handler: Callable[[JsonValue], JsonValue]

    def vertex_passthrough_handler(
        self, **kwargs: Unpack[HandlerCall]
    ) -> dict[str, JsonValue]: ...


@runtime_checkable
class LoggingSetup(Protocol):
    def update_environment_variables(
        self,
        litellm_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
    ) -> None: ...


class BranchHitError(Exception):
    def __init__(self, branch: str) -> None:
        super().__init__(branch)
        self.branch = branch


class Leaf:
    def __init__(self, branch: str) -> None:
        self.branch = branch

    def __call__(
        self,
        *_args: JsonValue,
        **_kwargs: JsonValue,
    ) -> JsonValue:
        raise BranchHitError(self.branch)


HANDLER: Final = TypeAdapter(VertexHandler, config=ARBITRARY)
LOGGING: Final = TypeAdapter(LoggingSetup, config=ARBITRARY)
INPUT: Final = TypeAdapter(Input)


def branch_for(url_route: str) -> str | None:
    handler_class = handler_module.VertexPassthroughLoggingHandler
    probe_class = type(
        "ProbePassthroughHandler",
        (handler_class,),
        {"interactions_passthrough_handler": Leaf("interactions")},
    )
    object.__setattr__(handler_module, "VertexPassthroughLoggingHandler", probe_class)
    handler = HANDLER.validate_python(handler_class)
    object.__setattr__(handler_module, "VertexAIVideoConfig", Leaf("video"))
    object.__setattr__(litellm, "ModelResponse", Leaf("generate_content"))
    object.__setattr__(embed_module, "process_embed_content_response", Leaf("embed_content"))
    object.__setattr__(embed_module, "process_response", Leaf("embed_content"))
    object.__setattr__(image_module, "VertexImageGeneration", Leaf("predict"))
    object.__setattr__(partner_module, "get_vertex_ai_partner_model_config", Leaf("raw_predict"))
    search = Leaf("search")
    object.__setattr__(search, "transform_search_vector_store_response", search)
    object.__setattr__(handler_module, "vertex_search_api_config", search)
    handler.batch_prediction_jobs_handler = Leaf("batch_prediction_jobs")
    logging_obj = Logging(
        model="",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id="b3642-call",
        function_id="b3642",
    )
    setup = LOGGING.validate_python(logging_obj)
    setup.update_environment_variables(litellm_params={}, optional_params={})
    reply = httpx.Response(200, content=b"{}")
    try:
        handler.vertex_passthrough_handler(
            httpx_response=reply,
            logging_obj=logging_obj,
            url_route=url_route,
            result="",
            start_time=START,
            end_time=START,
            cache_hit=False,
        )
    except BranchHitError as hit:
        return hit.branch
    return None


def run(input_value: Input) -> dict[str, JsonValue]:
    case = INPUT.validate_python(input_value)
    answer: dict[str, JsonValue] = {"branch": branch_for(case["url_route"])}
    return answer
