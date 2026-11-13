import base64
from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.common_utils import (
    _get_image_mime_type_from_url,  # pyright: ignore[reportPrivateUsage]
    extract_file_data,
    extract_file_metadata,
    infer_content_type_from_url_and_content,
)
from litellm.router_utils.batch_utils import InMemoryFile
from litellm.types.llms.openai import FileTypes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _file_body(wire: dict[str, Json]) -> bytes | str:
    if wire["kind"] == "bytes":
        return base64.b64decode(cast(str, wire["content_base64"]))
    return cast(str, wire["content"])


def _file_data(wire: dict[str, Json]) -> FileTypes:  # pyright: ignore[reportUnknownParameterType]
    kind = wire["kind"]
    if kind == "direct":
        return cast(FileTypes, _file_body(cast(dict[str, Json], wire["content"])))  # pyright: ignore[reportUnknownVariableType]
    if kind == "tuple2":
        return cast(  # pyright: ignore[reportUnknownVariableType]
            FileTypes,
            (
                cast("str | None", wire["filename"]),
                _file_body(cast(dict[str, Json], wire["content"])),
            ),
        )
    if kind == "tuple3":
        return cast(  # pyright: ignore[reportUnknownVariableType]
            FileTypes,
            (
                cast("str | None", wire["filename"]),
                _file_body(cast(dict[str, Json], wire["content"])),
                cast("str | None", wire["content_type"]),
            ),
        )
    if kind == "tuple4":
        return cast(  # pyright: ignore[reportUnknownVariableType]
            FileTypes,
            (
                cast("str | None", wire["filename"]),
                _file_body(cast(dict[str, Json], wire["content"])),
                cast("str | None", wire["content_type"]),
                cast("dict[str, str]", wire["headers"]),
            ),
        )
    content = base64.b64decode(cast(str, wire["content_base64"]))
    return cast(  # pyright: ignore[reportUnknownVariableType]
        FileTypes,
        InMemoryFile(content, cast(str, wire["name"]), cast(str, wire["content_type"])),
    )


def _extract_file_metadata(input_value: dict[str, Json]) -> Json:
    filename, content_type = extract_file_metadata(
        _file_data(cast(dict[str, Json], input_value["file_data"]))  # pyright: ignore[reportUnknownArgumentType]
    )
    return {"filename": filename, "content_type": content_type}


def _extract_file_data(input_value: dict[str, Json]) -> Json:
    extracted = extract_file_data(
        _file_data(cast(dict[str, Json], input_value["file_data"]))  # pyright: ignore[reportUnknownArgumentType]
    )
    return {
        "filename": extracted["filename"],
        "content_base64": base64.b64encode(extracted["content"]).decode("ascii"),
        "content_type": extracted["content_type"],
        "headers": dict(extracted["headers"]),
    }


def _image_mime_type_from_url(input_value: dict[str, Json]) -> Json:
    return _get_image_mime_type_from_url(cast(str, input_value["url"]))


def _infer_content_type_from_url_and_content(input_value: dict[str, Json]) -> Json:
    content = base64.b64decode(cast(str, input_value["content_base64"]))
    return infer_content_type_from_url_and_content(
        cast(str, input_value["url"]),
        content,
        cast("str | None", input_value.get("current_content_type")),
    )


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "extract_file_metadata": _extract_file_metadata,
    "extract_file_data": _extract_file_data,
    "image_mime_type_from_url": _image_mime_type_from_url,
    "infer_content_type_from_url_and_content": _infer_content_type_from_url_and_content,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return HANDLERS[operation](input_value)
