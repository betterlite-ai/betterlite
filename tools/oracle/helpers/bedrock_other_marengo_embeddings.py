import json
from collections.abc import Callable
from typing import Final, NotRequired, TypedDict

import litellm
from litellm.llms.bedrock.embed.twelvelabs_marengo_transformation import (
    TwelveLabsMarengoEmbeddingConfig,
    drop_params_enabled,
)
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

INVOCATION_ARN: Final = "invocationArn"


class Call(TypedDict):
    model: str
    input: list[str]
    inference: dict[str, JsonValue]
    replies: list[JsonValue]
    async_invoke_route: NotRequired[bool]
    model_id: NotRequired[str | None]
    output_s3_uri: NotRequired[str | None]
    drop_params: NotRequired[bool]
    batch: NotRequired[list[JsonValue]]
    global_drop_params: NotRequired[bool]
    deployment_drop_params: NotRequired[JsonValue]


class Config(TwelveLabsMarengoEmbeddingConfig):
    def request(self, call: Call, input_text: str) -> JsonValue:
        request = self._transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/twelvelabs_marengo_transformation.py#L182
            input_text,
            call["inference"],
            call.get("async_invoke_route") is True,
            call.get("model_id"),
            call.get("output_s3_uri"),
            call.get("drop_params") is True,
        )
        return TypeAdapter(JsonValue).validate_json(json.dumps(request))

    def replied(
        self,
        call: Call,
        replies: list[dict[str, JsonValue]],
        requests: list[dict[str, JsonValue]],
    ) -> EmbeddingResponse:
        return self._transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/twelvelabs_marengo_transformation.py#L310
            replies, call["model"], requests
        )

    def async_replied(self, call: Call, reply: dict[str, JsonValue]) -> EmbeddingResponse:
        return self._transform_async_invoke_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/twelvelabs_marengo_transformation.py#L321
            reply, call["model"]
        )


class EmbeddingItemView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    embedding: list[float]
    index: int
    object: str


class DetailsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    query_count: int | None = None
    image_count: int | None = None


class UsageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0
    prompt_tokens_details: DetailsView | None = None


class HiddenView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    invocation_arn: JsonValue = Field(default=None, alias="_invocation_arn")


class ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    data: list[EmbeddingItemView]
    model: str | None = None
    usage: UsageView | None = None
    hidden: HiddenView = Field(alias="_hidden_params")


class EmbeddingOut(BaseModel):
    embedding: list[float]
    index: int
    object: str


class DetailsOut(BaseModel):
    query_count: int | None
    image_count: int | None


class UsageOut(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: DetailsOut | None


class ResponseOut(BaseModel):
    data: list[EmbeddingOut]
    model: str | None
    usage: UsageOut
    invocation_arn: JsonValue


class TransformOut(BaseModel):
    supported: list[str]
    requests: list[JsonValue]
    response: ResponseOut


class DropOut(BaseModel):
    supported: list[str]
    enabled: bool


def response_out(response: EmbeddingResponse) -> ResponseOut:
    view = ResponseView.model_validate(response)
    usage = view.usage
    details = usage.prompt_tokens_details if usage is not None else None
    return ResponseOut(
        data=[
            EmbeddingOut(embedding=item.embedding, index=item.index, object=item.object)
            for item in view.data
        ],
        model=view.model,
        usage=UsageOut(
            prompt_tokens=0 if usage is None else usage.prompt_tokens,
            completion_tokens=0 if usage is None else usage.completion_tokens,
            total_tokens=0 if usage is None else usage.total_tokens,
            prompt_tokens_details=None
            if details is None
            else DetailsOut(query_count=details.query_count, image_count=details.image_count),
        ),
        invocation_arn=view.hidden.invocation_arn,
    )


def response_for(
    call: Call,
    config: Config,
    replies: list[dict[str, JsonValue]],
    requests: list[dict[str, JsonValue]],
) -> ResponseOut:
    asynchronous = call.get("async_invoke_route") is True
    if asynchronous and len(replies) == 1 and INVOCATION_ARN in replies[0]:
        return response_out(config.async_replied(call, replies[0]))
    return response_out(config.replied(call, replies, requests))


def transform_out(call: Call) -> JsonValue:
    config = Config(model=call["model"])
    requests = [config.request(call, text) for text in call["input"]]
    reply_values = TypeAdapter(list[dict[str, JsonValue]]).validate_python(call["replies"])
    batch_values = (
        TypeAdapter(list[dict[str, JsonValue]]).validate_python(call["batch"])
        if "batch" in call
        else TypeAdapter(list[dict[str, JsonValue]]).validate_python(requests)
    )
    response = response_for(
        call,
        config,
        reply_values[: len(call["input"])],
        batch_values,
    )
    out = TransformOut(
        supported=config.get_supported_openai_params(),
        requests=requests,
        response=response,
    )
    return TypeAdapter(JsonValue).validate_json(out.model_dump_json())


def drop_out(call: Call) -> JsonValue:
    litellm.drop_params = call.get("global_drop_params") is True
    mapping: dict[str, JsonValue] = {"drop_params": call.get("deployment_drop_params")}
    config = Config(model=call["model"])
    out = DropOut(
        supported=config.get_supported_openai_params(),
        enabled=drop_params_enabled(mapping),
    )
    return TypeAdapter(JsonValue).validate_json(out.model_dump_json())


HANDLERS: Final[dict[str, Callable[[Call], JsonValue]]] = {
    "marengo_text_input": transform_out,
    "marengo_s3_video_owner": transform_out,
    "marengo_image_input": transform_out,
    "marengo_async_output_wrapper": transform_out,
    "marengo_segments_vector_list": transform_out,
    "marengo_billed_units": transform_out,
    "drop_unsupported_param": drop_out,
}


def run(input_value: dict[str, Call]) -> JsonValue:
    name, call = next(iter(input_value.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown bedrock_other_marengo_embeddings case: {name}")
    return handler(call)
