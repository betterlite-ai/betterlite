import json
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
SCHEMA_FIELD = "bedrock_invocation_schema"

litellm.suppress_debug_info = True


class ClassifyRecords(TypedDict):
    function: Literal["classify_records"]
    records: list[dict[str, JsonValue]]


class RegistrySchema(TypedDict):
    function: Literal["registry_schema"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class NameBoundary(TypedDict):
    function: Literal["name_boundary"]
    models: list[str]


class CoerceInput(TypedDict):
    function: Literal["coerce_input"]
    model: str
    input: NotRequired[JsonValue]


class MapEmbedding(TypedDict):
    function: Literal["map_embedding"]
    model_cost: dict[str, JsonValue]
    model: str
    body: dict[str, JsonValue]


Input = ClassifyRecords | RegistrySchema | NameBoundary | CoerceInput | MapEmbedding


# every owned function is private, so the subclass reaches it through self and nothing public is
class BatchEmbeddingConfig(BedrockFilesConfig):
    def classify(self, record: dict[str, JsonValue]) -> JsonValue:
        kind = self._classify_batch_record(record)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L575
        return JSON.validate_json(json.dumps(kind.value))  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/bedrock.py#L1250

    def registry(self, model: str) -> JsonValue:
        schema = self._lookup_provider_specific_field(model, SCHEMA_FIELD)
        titan = self._is_titan_v2_embed_model(model)
        answer: dict[str, JsonValue] = {"schema": schema, "titan": titan}
        return JSON.validate_json(json.dumps(answer))

    def titan(self, model: str) -> JsonValue:
        return JSON.validate_json(json.dumps(self._is_titan_v2_embed_model(model)))

    def coerce(self, raw: JsonValue, model: str) -> JsonValue:
        coerced = self._coerce_embedding_input_to_string(raw, model=model)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L718
        return JSON.validate_json(json.dumps(coerced))

    def mapping(self, body: dict[str, JsonValue], model: str) -> JsonValue:
        mapped = self._map_openai_embedding_to_bedrock_params(body, model)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L773
        return JSON.validate_json(json.dumps(mapped))


def classify_records(config: BatchEmbeddingConfig, call: ClassifyRecords) -> JsonValue:
    kinds: list[JsonValue] = [config.classify(record) for record in call["records"]]
    answer: dict[str, JsonValue] = {"kinds": kinds}
    return JSON.validate_json(json.dumps(answer))


def registry_schema(config: BatchEmbeddingConfig, call: RegistrySchema) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = [config.registry(model) for model in call["models"]]
    answer: dict[str, JsonValue] = {"results": results}
    return JSON.validate_json(json.dumps(answer))


def name_boundary(config: BatchEmbeddingConfig, call: NameBoundary) -> JsonValue:
    litellm.model_cost = {}
    results: list[JsonValue] = [config.titan(model) for model in call["models"]]
    answer: dict[str, JsonValue] = {"results": results}
    return JSON.validate_json(json.dumps(answer))


def coerce_input(config: BatchEmbeddingConfig, call: CoerceInput) -> JsonValue:
    coerced = config.coerce(call.get("input"), call["model"])
    answer: dict[str, JsonValue] = {"coerced": coerced}
    return JSON.validate_json(json.dumps(answer))


def map_embedding(config: BatchEmbeddingConfig, call: MapEmbedding) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    return config.mapping(call["body"], call["model"])


def run(input_value: Input) -> JsonValue:
    config = BatchEmbeddingConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
    if input_value["function"] == "classify_records":
        return classify_records(config, input_value)
    if input_value["function"] == "registry_schema":
        return registry_schema(config, input_value)
    if input_value["function"] == "name_boundary":
        return name_boundary(config, input_value)
    if input_value["function"] == "coerce_input":
        return coerce_input(config, input_value)
    return map_embedding(config, input_value)
