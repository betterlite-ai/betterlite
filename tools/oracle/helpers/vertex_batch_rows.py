from collections.abc import Callable, Mapping
from typing import Literal, TypedDict, cast

from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.vertex_ai.files import transformation
from litellm.llms.vertex_ai.files.transformation import is_passthrough_managed_gcs_url
from pydantic import JsonValue

# LiteLLM reaches these row functions only through private names, bound here in one line
SET_LABELS, READ_LABELS, SPLIT_KEY, IS_EMBEDDING, IS_GENERATE, MODEL_OF, TRANSFORM = cast(tuple[Callable[[dict[str, str], str], None], Callable[[Mapping[str, JsonValue]], str], Callable[[Mapping[str, JsonValue]], tuple[str, int, int]], Callable[[Mapping[str, JsonValue]], bool], Callable[[Mapping[str, JsonValue]], bool], Callable[[str], str | None], Callable[[list[Mapping[str, JsonValue]], str | None], list[JsonValue]]], (transformation._set_litellm_batch_custom_id_labels, transformation._get_litellm_batch_custom_id_from_labels, transformation._split_vertex_batch_key, transformation._is_vertex_embeddings_batch_output_row, transformation._is_vertex_generate_content_batch_output_row, transformation._model_from_managed_gcs_url, transformation._transform_vertex_embeddings_batch_output_to_openai))  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/transformation.py#L159


class FixedUuid:
    def __init__(self) -> None:
        self.issued = 0

    def uuid4(self) -> UUID:
        self.issued += 1
        return UUID(int=self.issued)


litellm_uuid.uuid4 = FixedUuid().uuid4


class EncodeInput(TypedDict):
    operation: Literal["encode"]
    custom_ids: list[str]


class RowsInput(TypedDict):
    operation: Literal["rows"]
    rows: list[dict[str, JsonValue]]


class RegroupInput(TypedDict):
    operation: Literal["regroup"]
    rows: list[dict[str, JsonValue]]
    model: str | None


class UrlsInput(TypedDict):
    operation: Literal["urls"]
    urls: list[str]


Input = EncodeInput | RowsInput | RegroupInput | UrlsInput


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "encode":
        encoded: list[JsonValue] = []
        for custom_id in input_value["custom_ids"]:
            labels: dict[str, str] = {}
            SET_LABELS(labels, custom_id)
            encoded.append({"labels": {**labels}, "custom_id": READ_LABELS(labels)})
        return {"rows": encoded}
    if input_value["operation"] == "rows":
        found: list[JsonValue] = []
        for row in input_value["rows"]:
            custom_id, index, total = SPLIT_KEY(row)
            found.append({
                "custom_id": custom_id,
                "embeddings": IS_EMBEDDING(row),
                "generate": IS_GENERATE(row),
                "index": index,
                "total": total,
            })
        return {"rows": found}
    if input_value["operation"] == "regroup":
        inputs: list[Mapping[str, JsonValue]] = list(input_value["rows"])
        return {"rows": list(TRANSFORM(inputs, input_value["model"]))}
    urls: list[JsonValue] = [
        {"passthrough": is_passthrough_managed_gcs_url(url), "model": MODEL_OF(url)}
        for url in input_value["urls"]
    ]
    return {"rows": urls}
