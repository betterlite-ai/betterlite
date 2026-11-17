import json
from typing import Protocol

from litellm.llms.bedrock.embed.amazon_titan_g1_transformation import AmazonTitanG1Config
from litellm.llms.bedrock.embed.amazon_titan_multimodal_transformation import (
    AmazonTitanMultimodalEmbeddingG1Config as MultimodalConfig,
)
from litellm.llms.bedrock.embed.amazon_titan_v2_transformation import AmazonTitanV2Config
from litellm.llms.bedrock.embed.cohere_transformation import BedrockCohereEmbeddingConfig
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


# get_config has no return annotation upstream, so the protocol gives the probes its signature
class ConfigProbe(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, JsonValue]: ...


class Mapper(Protocol):
    def map_openai_params(
        self, non_default_params: dict[str, JsonValue], optional_params: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...


class Supported(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...


# an embedding transform both declares its supported OpenAI params and maps them
class EmbeddingConfig(Mapper, Supported, Protocol):
    ...


# the source map mutates the record it is given, so every call hands it its own copy
def mapped(
    config: Mapper, non_default: dict[str, JsonValue], optional_params: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = dict(optional_params)
    return config.map_openai_params(dict(non_default), params)


def supported(config: EmbeddingConfig) -> JsonValue:
    names = config.get_supported_openai_params()
    return JSON.validate_python(names)


# every transform reports the same supported, mapped and non_default surface
def params_report(
    config: EmbeddingConfig, non_default: dict[str, JsonValue], params: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    names = supported(config)
    return {"supported": names, "mapped": params, "non_default": non_default}


class TitanG1(AmazonTitanG1Config):
    def request(self, input_text: str, inference_params: dict[str, JsonValue]) -> JsonValue:
        request = self._transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/amazon_titan_g1_transformation.py#L59-L60
            input_text, inference_params
        )
        return JSON.validate_json(json.dumps(request))

    def replied(self, response_list: list[dict[str, JsonValue]], model: str) -> EmbeddingResponse:
        return self._transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/amazon_titan_g1_transformation.py#L62-L82
            response_list, model
        )


class TitanV2(AmazonTitanV2Config):
    def request(self, input_text: str, inference_params: dict[str, JsonValue]) -> JsonValue:
        request = self._transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/amazon_titan_v2_transformation.py#L76-L77
            input_text, inference_params
        )
        return JSON.validate_json(json.dumps(request))

    def replied(self, response_list: list[dict[str, JsonValue]], model: str) -> EmbeddingResponse:
        return self._transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/amazon_titan_v2_transformation.py#L79-L117
            response_list, model
        )


class TitanMultimodal(MultimodalConfig):
    def request(self, input_text: str, inference_params: dict[str, JsonValue]) -> JsonValue:
        request = self._transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/amazon_titan_multimodal_transformation.py#L42-L53
            input_text, inference_params
        )
        return JSON.validate_json(json.dumps(request))

    def replied(
        self,
        response_list: list[dict[str, JsonValue]],
        model: str,
        batch_data: list[dict[str, JsonValue]] | None,
    ) -> EmbeddingResponse:
        return self._transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/amazon_titan_multimodal_transformation.py#L55-L93
            response_list, model, batch_data
        )


class TitanCohere(BedrockCohereEmbeddingConfig):
    def request(
        self, model: str, input_texts: list[str], inference_params: dict[str, JsonValue]
    ) -> JsonValue:
        request = self._transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/cohere_transformation.py#L33-L43
            model, input_texts, inference_params
        )
        return JSON.validate_json(json.dumps(request))
