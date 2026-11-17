from dataclasses import dataclass, field
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.vertex_ai.vertex_ai_partner_models.anthropic.transformation import (
    VertexAIAnthropicConfig,
)
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue as Json, TypeAdapter
from typing_extensions import override

BETA_LIST: Final = TypeAdapter(list[str])


class Fixture(TypedDict):
    model_cost: dict[str, Json]
    packaged: dict[str, Json]


class MapStep(TypedDict):
    function: Literal["map_openai_params"]
    model: str
    non_default_params: dict[str, Json]
    optional_params: dict[str, Json]
    drop_params: bool
    global_drop_params: bool


class RequestStep(TypedDict):
    function: Literal["transform_request"]
    model: str
    messages: list[Json]
    optional_params: dict[str, Json]
    headers: dict[str, str]
    drop_params: bool
    global_drop_params: bool


class ResponseStep(TypedDict):
    function: Literal["transform_response"]
    model: str
    status: int
    raw_text: str
    headers: dict[str, str]
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    request_data: dict[str, Json]
    json_mode: bool | None


Step: TypeAlias = MapStep | RequestStep | ResponseStep


class Input(TypedDict):
    fixture: Fixture
    steps: list[Step]


@runtime_checkable
class ParamMapper(Protocol):
    def map_openai_params(
        self,
        *,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        drop_params: bool,
    ) -> dict[str, Json]: ...


@runtime_checkable
class Transformer(Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        headers: dict[str, str],
    ) -> dict[str, Json]: ...


@runtime_checkable
class LoggingSink(Protocol):
    def post_call(self, **kwargs: Json) -> None: ...


@runtime_checkable
class Responder(Protocol):
    def transform_response(  # noqa: E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L2709-L2722
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: LoggingSink,
        request_data: dict[str, Json],
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        encoding: None,
        *,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


class Bound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    mapper: ParamMapper
    transformer: Transformer
    responder: Responder


@dataclass
class Logging(LoggingSink):
    kwargs: dict[str, Json] = field(default_factory=dict)

    @override
    def post_call(self, **kwargs: Json) -> None:
        self.kwargs = kwargs


def map_answer(config: ParamMapper, step: MapStep) -> Json:
    litellm.drop_params = step["global_drop_params"]
    mapped = config.map_openai_params(
        non_default_params=step["non_default_params"],
        optional_params=dict(step["optional_params"]),
        model=step["model"],
        drop_params=step["drop_params"],
    )
    return {"optional_params": mapped}


def request_answer(transformer: Transformer, step: RequestStep) -> Json:
    litellm.drop_params = step["global_drop_params"]
    litellm.modify_params = False
    optional_params: dict[str, Json] = dict(step["optional_params"])
    # the partner handler marks the request before the anthropic handler builds it
    optional_params["is_vertex_request"] = True
    litellm_params: dict[str, Json] = {}
    if step["drop_params"]:
        litellm_params["drop_params"] = True
    headers: dict[str, str] = dict(step["headers"])
    body = transformer.transform_request(
        model=step["model"],
        messages=list(step["messages"]),
        optional_params=optional_params,
        litellm_params=litellm_params,
        headers=headers,
    )
    betas = sorted(BETA_LIST.validate_python(body.pop("anthropic_beta", [])))
    beta_header = [part for part in headers.pop("anthropic-beta", "").split(",") if part]
    return {"body": body, "anthropic_beta": betas, "beta_header": sorted(beta_header)}


def response_answer(responder: Responder, step: ResponseStep) -> Json:
    response = responder.transform_response(
        model=step["model"],
        raw_response=httpx.Response(
            step["status"],
            content=step["raw_text"].encode(),
            headers=step["headers"],
        ),
        model_response=ModelResponse(id="chatcmpl-b1333"),
        logging_obj=Logging(),
        request_data=step["request_data"],
        messages=list(step["messages"]),
        optional_params=dict(step["optional_params"]),
        litellm_params=dict(step["litellm_params"]),
        encoding=None,
        json_mode=step["json_mode"],
    )
    return {"model": response.model}


def answer(step: Step) -> Json:
    config = VertexAIAnthropicConfig()
    holder: dict[str, VertexAIAnthropicConfig] = {
        "mapper": config,
        "transformer": config,
        "responder": config,
    }
    bound = Bound.model_validate(holder)
    if step["function"] == "map_openai_params":
        return map_answer(bound.mapper, step)
    if step["function"] == "transform_request":
        return request_answer(bound.transformer, step)
    return response_answer(bound.responder, step)


def run(input_value: Input) -> list[Json]:
    litellm.model_cost = input_value["fixture"]["model_cost"]
    # the proxy and Router set this, else an unknown model prints a banner to stdout
    litellm.suppress_debug_info = True
    return [answer(step) for step in input_value["steps"]]
