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

from b3633 import make_logging

START: Final = datetime.fromisoformat("2026-01-01T00:00:00+00:00")
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class Input(TypedDict):
    url_route: str


class SearchConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_response(
        self, **_kwargs: object  # ast-grep-ignore: no-loose-types
    ) -> JsonValue: ...


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
    interactions_passthrough_handler: Callable[..., JsonValue]  # ast-grep-ignore
    batch_prediction_jobs_handler: Callable[..., JsonValue]

    def vertex_passthrough_handler(
        self, **kwargs: Unpack[HandlerCall]
    ) -> dict[str, JsonValue]: ...


@runtime_checkable
class HandlerModule(Protocol):
    VertexAIVideoConfig: Callable[..., JsonValue]
    vertex_search_api_config: SearchConfig


@runtime_checkable
class LitellmModule(Protocol):
    ModelResponse: Callable[..., JsonValue]


@runtime_checkable
class ImageModule(Protocol):
    VertexImageGeneration: Callable[..., JsonValue]


@runtime_checkable
class EmbedModule(Protocol):
    process_embed_content_response: Callable[..., JsonValue]
    process_response: Callable[..., JsonValue]


@runtime_checkable
class PartnerModule(Protocol):
    get_vertex_ai_partner_model_config: Callable[..., JsonValue]  # ast-grep-ignore


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
        *_args: object,  # ast-grep-ignore: no-loose-types
        **_kwargs: object,  # ast-grep-ignore: no-loose-types
    ) -> JsonValue:
        raise BranchHitError(self.branch)


class SearchLeaf(SearchConfig):
    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_response(
        self, **_kwargs: object  # ast-grep-ignore: no-loose-types
    ) -> JsonValue:
        raise BranchHitError("search")


HANDLER: Final = TypeAdapter(VertexHandler, config=ARBITRARY)
HANDLER_MODULE: Final = TypeAdapter(HandlerModule, config=ARBITRARY)
LITELLM: Final = TypeAdapter(LitellmModule, config=ARBITRARY)
IMAGES: Final = TypeAdapter(ImageModule, config=ARBITRARY)
EMBEDDINGS: Final = TypeAdapter(EmbedModule, config=ARBITRARY)
PARTNERS: Final = TypeAdapter(PartnerModule, config=ARBITRARY)
LOGGING: Final = TypeAdapter(LoggingSetup, config=ARBITRARY)
INPUT: Final = TypeAdapter(Input)


def branch_for(url_route: str) -> str | None:
    handler = HANDLER.validate_python(handler_module.VertexPassthroughLoggingHandler)
    bindings = HANDLER_MODULE.validate_python(handler_module)
    core = LITELLM.validate_python(litellm)
    images = IMAGES.validate_python(image_module)
    embeddings = EMBEDDINGS.validate_python(embed_module)
    partners = PARTNERS.validate_python(partner_module)
    handler.interactions_passthrough_handler = Leaf("interactions")
    bindings.VertexAIVideoConfig = Leaf("video")
    core.ModelResponse = Leaf("generate_content")
    embeddings.process_embed_content_response = Leaf("embed_content")
    embeddings.process_response = Leaf("embed_content")
    images.VertexImageGeneration = Leaf("predict")
    partners.get_vertex_ai_partner_model_config = Leaf("raw_predict")
    bindings.vertex_search_api_config = SearchLeaf()
    handler.batch_prediction_jobs_handler = Leaf("batch_prediction_jobs")
    logging_obj = make_logging(START)
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
    return {"branch": branch_for(case["url_route"])}
