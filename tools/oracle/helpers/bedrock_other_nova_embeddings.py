import json
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

from litellm.llms.bedrock.embed.amazon_nova_transformation import AmazonNovaEmbeddingConfig
from litellm.types.utils import Embedding, EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue

JsonMap: TypeAlias = dict[str, JsonValue]


class NovaConfig(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...
    def map_openai_params(
        self, non_default_params: JsonMap, optional_params: JsonMap
    ) -> JsonMap: ...
    def _parse_data_url(self, data_url: str) -> tuple[str, str]: ...
    def _transform_request(
        self,
        input: str,  # noqa: A002
        inference_params: JsonMap,
        async_invoke_route: bool = False,  # noqa: FBT001, FBT002
        model_id: str | None = None,
        output_s3_uri: str | None = None,
    ) -> JsonMap: ...
    def _transform_response(
        self, response_list: list[JsonMap], model: str, batch_data: list[JsonMap] | None = None
    ) -> EmbeddingResponse: ...
    def _transform_async_invoke_response(
        self, response: JsonMap, model: str
    ) -> EmbeddingResponse: ...


def as_nova_config() -> NovaConfig:
    return cast(NovaConfig, AmazonNovaEmbeddingConfig())


class DefaultPurposeCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    non_default: JsonMap = {}
    optional_params: JsonMap = {}
    input: str = ""
    inference_params: JsonMap = {}
    async_invoke_route: bool = False
    model_id: str | None = None
    output_s3_uri: str | None = None


class TransformRequestCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    input: str = ""
    inference_params: JsonMap = {}
    async_invoke_route: bool = False
    model_id: str | None = None
    output_s3_uri: str | None = None


class DataUrlCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    data_url: str = ""


class TransformResponseCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    response_list: list[JsonMap] = []
    batch_data: list[JsonMap] | None = None


class InvocationArnCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    response: JsonMap = {}


def to_json(value: object) -> JsonValue:
    return cast("JsonValue", json.loads(json.dumps(value)))


def embedding_response_json(response: EmbeddingResponse) -> JsonValue:
    usage = response.usage
    details = usage.prompt_tokens_details if usage else None
    hidden_params = cast(
        "dict[str, JsonValue]",
        response._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    invocation_arn = hidden_params.get("_invocation_arn")
    items = cast("list[Embedding]", response.data)
    embeddings = [cast("JsonValue", item.embedding) for item in items]
    return to_json(
        {
            "data": embeddings,
            "model": response.model,
            "usage": {
                "prompt_tokens": usage.prompt_tokens if usage else 0,
                "total_tokens": usage.total_tokens if usage else 0,
                "prompt_tokens_details": (
                    {"image_count": details.image_count} if details is not None else None
                ),
            },
            "invocation_arn": invocation_arn,
        }
    )


def nova_default_purpose_dimensions(argument: JsonValue) -> JsonValue:
    call = DefaultPurposeCall.model_validate(argument)
    config = as_nova_config()
    supported = config.get_supported_openai_params()
    mapped = config.map_openai_params(dict(call.non_default), dict(call.optional_params))
    request = config._transform_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        call.input,
        dict(call.inference_params),
        call.async_invoke_route,
        call.model_id,
        call.output_s3_uri,
    )
    return to_json({"supported_params": supported, "mapped": mapped, "request": request})


def run_transform_request(argument: JsonValue) -> JsonValue:
    call = TransformRequestCall.model_validate(argument)
    config = as_nova_config()
    request = config._transform_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        call.input,
        dict(call.inference_params),
        call.async_invoke_route,
        call.model_id,
        call.output_s3_uri,
    )
    return to_json(request)


def nova_data_url_jpg(argument: JsonValue) -> JsonValue:
    call = DataUrlCall.model_validate(argument)
    config = as_nova_config()
    media_type, data = config._parse_data_url(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        call.data_url
    )
    is_async = False
    request = config._transform_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        call.data_url, {}, is_async, None, None
    )
    return to_json({"media_type": media_type, "data": data, "request": request})


def nova_result_index_token_estimate(argument: JsonValue) -> JsonValue:
    call = TransformResponseCall.model_validate(argument)
    config = as_nova_config()
    batch_data = list(call.batch_data) if call.batch_data is not None else None
    response = config._transform_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        list(call.response_list), "nova-model", batch_data
    )
    return embedding_response_json(response)


def nova_invocation_arn(argument: JsonValue) -> JsonValue:
    call = InvocationArnCall.model_validate(argument)
    config = as_nova_config()
    response = config._transform_async_invoke_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        dict(call.response), "nova-model"
    )
    return embedding_response_json(response)


HANDLERS: dict[str, Callable[[JsonValue], JsonValue]] = {
    "nova_default_purpose_dimensions": nova_default_purpose_dimensions,
    "nova_text_image_audio_video": run_transform_request,
    "nova_data_url_jpg": nova_data_url_jpg,
    "nova_explicit_modality": run_transform_request,
    "nova_async_output_s3_required": run_transform_request,
    "nova_result_index_token_estimate": nova_result_index_token_estimate,
    "nova_invocation_arn": nova_invocation_arn,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    name, argument = next(iter(input_value.items()))
    return HANDLERS[name](argument)
