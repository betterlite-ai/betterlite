from collections.abc import Callable, Mapping
from typing import Final, Literal, NamedTuple, TypedDict, cast

from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.vertex_ai.files import transformation
from litellm.llms.vertex_ai.files.transformation import is_passthrough_managed_gcs_url
from pydantic import JsonValue, TypeAdapter

LABELS: Final = TypeAdapter(dict[str, JsonValue])


class Handlers(NamedTuple):
    set_labels: Callable[[dict[str, str], str], None]
    read_labels: Callable[[Mapping[str, JsonValue]], str]
    read_id: Callable[[Mapping[str, JsonValue]], str]
    split_key: Callable[[Mapping[str, JsonValue]], tuple[str, int, int]]
    is_embedding: Callable[[Mapping[str, JsonValue]], bool]
    is_generate: Callable[[Mapping[str, JsonValue]], bool]
    model_of: Callable[[str], str | None]
    transform: Callable[[list[Mapping[str, JsonValue]], str | None], list[JsonValue]]


RowFunctions = tuple[
    Callable[[dict[str, str], str], None],
    Callable[[Mapping[str, JsonValue]], str],
    Callable[[Mapping[str, JsonValue]], str],
    Callable[[Mapping[str, JsonValue]], tuple[str, int, int]],
    Callable[[Mapping[str, JsonValue]], bool],
    Callable[[Mapping[str, JsonValue]], bool],
    Callable[[str], str | None],
    Callable[[list[Mapping[str, JsonValue]], str | None], list[JsonValue]],
]

# LiteLLM reaches these row functions only through private names, bound here in one line
BOUND: Final[RowFunctions] = cast(RowFunctions, (transformation._set_litellm_batch_custom_id_labels, transformation._get_litellm_batch_custom_id_from_labels, transformation._get_litellm_batch_custom_id, transformation._split_vertex_batch_key, transformation._is_vertex_embeddings_batch_output_row, transformation._is_vertex_generate_content_batch_output_row, transformation._model_from_managed_gcs_url, transformation._transform_vertex_embeddings_batch_output_to_openai))  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/transformation.py#L159
HANDLERS: Final[Handlers] = Handlers(*BOUND)


class FixedUuid:
    def __init__(self) -> None:
        self.count = 0

    def uuid4(self) -> UUID:
        self.count += 1
        return UUID(int=self.count)


litellm_uuid.uuid4 = FixedUuid().uuid4


class EncodeInput(TypedDict):
    operation: Literal["encode"]
    custom_ids: list[str]


class ResolveInput(TypedDict):
    operation: Literal["resolve"]
    rows: list[dict[str, JsonValue]]


class SplitInput(TypedDict):
    operation: Literal["split"]
    rows: list[dict[str, JsonValue]]


class RegroupInput(TypedDict):
    operation: Literal["regroup"]
    rows: list[dict[str, JsonValue]]
    model: str | None


class UrlsInput(TypedDict):
    operation: Literal["urls"]
    urls: list[str]


Input = (
    EncodeInput
    | ResolveInput
    | SplitInput
    | RegroupInput
    | UrlsInput
)


def encode(fields: EncodeInput) -> JsonValue:
    rows: list[JsonValue] = []
    for custom_id in fields["custom_ids"]:
        labels: dict[str, str] = {}
        HANDLERS.set_labels(labels, custom_id)
        written: dict[str, JsonValue] = LABELS.validate_python(labels)
        row: dict[str, JsonValue] = {
            "labels": written,
            "custom_id": HANDLERS.read_labels(labels),
        }
        rows.append(row)
    result: dict[str, JsonValue] = {"rows": rows}
    return result


def resolve(fields: ResolveInput) -> JsonValue:
    rows: list[JsonValue] = []
    for row in fields["rows"]:
        found: dict[str, JsonValue] = {
            "custom_id": HANDLERS.read_id(row),
            "embeddings": HANDLERS.is_embedding(row),
            "generate": HANDLERS.is_generate(row),
        }
        rows.append(found)
    result: dict[str, JsonValue] = {"rows": rows}
    return result


def split(fields: SplitInput) -> JsonValue:
    rows: list[JsonValue] = []
    for row in fields["rows"]:
        custom_id, index, total = HANDLERS.split_key(row)
        found: list[JsonValue] = [custom_id, index, total]
        rows.append(found)
    result: dict[str, JsonValue] = {"rows": rows}
    return result


def regroup(fields: RegroupInput) -> JsonValue:
    inputs: list[Mapping[str, JsonValue]] = list(fields["rows"])
    rows: list[JsonValue] = list(HANDLERS.transform(inputs, fields["model"]))
    result: dict[str, JsonValue] = {"rows": rows}
    return result


def urls(fields: UrlsInput) -> JsonValue:
    rows: list[JsonValue] = [
        {
            "passthrough": is_passthrough_managed_gcs_url(url),
            "model": HANDLERS.model_of(url),
        }
        for url in fields["urls"]
    ]
    result: dict[str, JsonValue] = {"rows": rows}
    return result


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "encode":
        return encode(input_value)
    if input_value["operation"] == "resolve":
        return resolve(input_value)
    if input_value["operation"] == "split":
        return split(input_value)
    if input_value["operation"] == "regroup":
        return regroup(input_value)
    return urls(input_value)
