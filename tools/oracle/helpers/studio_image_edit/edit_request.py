import base64
import io
import pathlib
from typing import Annotated, Literal, NotRequired, TypeAlias, TypedDict

from pydantic import Field, JsonValue

from studio_image_edit.edit_config import (
    CONFIG,
    DROP_PARAMS,
    ImageInput,
    Stored,
    apply_environment,
)
from studio_image_edit.edit_values import JSON, JsonObject


class BytesCase(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


class SeekableCase(TypedDict):
    kind: Literal["seekable"]
    data_base64: str
    position: int


class UnsupportedCase(TypedDict):
    kind: Literal["unsupported"]


ImageCase: TypeAlias = Annotated[
    BytesCase | SeekableCase | UnsupportedCase,
    Field(discriminator="kind"),
]


class OneField(TypedDict):
    kind: Literal["one"]
    record: ImageCase


class ManyField(TypedDict):
    kind: Literal["many"]
    records: list[ImageCase | None]


ImageField: TypeAlias = Annotated[OneField | ManyField, Field(discriminator="kind")]


class RequestCase(TypedDict):
    model: str
    prompt: NotRequired[str | None]
    image: NotRequired[ImageField | None]
    params: NotRequired[JsonObject]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    env: NotRequired[dict[str, str]]


class PreparedImage(TypedDict):
    record: Stored
    cursor: io.BytesIO | None


class PreparedField(TypedDict):
    kind: Literal["one", "many"]
    records: list[PreparedImage | None]


def prepared_image(spec: ImageCase) -> PreparedImage:
    match spec["kind"]:
        case "bytes":
            return {"record": base64.b64decode(spec["data_base64"]), "cursor": None}
        case "seekable":
            stream = io.BytesIO(base64.b64decode(spec["data_base64"]))
            stream.seek(spec["position"])
            return {"record": stream, "cursor": stream}
        case "unsupported":
            return {"record": pathlib.Path("/images/input.png"), "cursor": None}


def prepared_field(field: ImageField | None) -> PreparedField | None:
    if field is None:
        return None
    if field["kind"] == "one":
        return {"kind": "one", "records": [prepared_image(field["record"])]}
    return {
        "kind": "many",
        "records": [
            None if spec is None else prepared_image(spec) for spec in field["records"]
        ],
    }


def stored_value(prepared: PreparedImage | None) -> Stored | None:
    if prepared is None:
        return None
    return prepared["record"]


def images_value(prepared: PreparedField | None) -> ImageInput:
    if prepared is None:
        return None
    if prepared["kind"] == "one":
        return stored_value(prepared["records"][0])
    return [stored_value(item) for item in prepared["records"]]


def cursor_position(prepared: PreparedImage | None) -> JsonValue:
    if prepared is None:
        return None
    cursor = prepared["cursor"]
    return None if cursor is None else cursor.tell()


def positions_value(prepared: PreparedField | None) -> list[JsonValue]:
    if prepared is None:
        return []
    return [cursor_position(item) for item in prepared["records"]]


def request_value(case: RequestCase) -> JsonObject:
    apply_environment(case.get("env"))
    model = case["model"]
    given_params = case.get("params")
    params: JsonObject = {} if given_params is None else given_params
    mapped = CONFIG.map_openai_params(params, model, DROP_PARAMS)
    supported: JsonValue = JSON.validate_python(CONFIG.get_supported_openai_params(model))
    multipart = CONFIG.use_multipart_form_data()
    api_base = case.get("api_base")
    url = CONFIG.get_complete_url(model, api_base, {})
    given_headers = case.get("headers")
    headers = CONFIG.validate_environment(
        {} if given_headers is None else dict(given_headers),
        model,
        case.get("api_key"),
        {},
        api_base,
    )
    prepared = prepared_field(case.get("image"))
    payload, files = CONFIG.transform_image_edit_request(
        model, case.get("prompt"), images_value(prepared), mapped, {}, {}
    )
    return {
        "url": url,
        "headers": headers,
        "supported": supported,
        "multipart": multipart,
        "mapped": mapped,
        "payload": payload,
        "files": len(files),
        "positions": positions_value(prepared),
    }
