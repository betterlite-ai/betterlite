from typing import Literal, TypedDict

from litellm.types.files import (
    FileType,
    get_file_extension_from_mime_type,
    get_file_mime_type_from_extension,
    is_gemini_1_5_accepted_file_type,
)


class Input(TypedDict):
    function: Literal["mime_extension_table", "mime_lookup_edges", "gemini_file_acceptance"]
    extensions: list[str]
    mimes: list[str]
    file_types: list[str]


def _mime_table(input_value: Input) -> list[object]:
    return [
        {"extension": extension, "mime": get_file_mime_type_from_extension(extension)}
        for extension in input_value["extensions"]
    ]


def _lookup_edges(input_value: Input) -> dict[str, object]:
    return {
        "extensions": [
            get_file_mime_type_from_extension(extension) for extension in input_value["extensions"]
        ],
        "mimes": [get_file_extension_from_mime_type(mime) for mime in input_value["mimes"]],
    }


def _gemini_acceptance(input_value: Input) -> list[object]:
    return [
        is_gemini_1_5_accepted_file_type(FileType(name)) for name in input_value["file_types"]
    ]


def run(input_value: Input) -> dict[str, object] | list[object]:
    function = input_value["function"]
    if function == "mime_extension_table":
        return _mime_table(input_value)
    if function == "mime_lookup_edges":
        return _lookup_edges(input_value)
    return _gemini_acceptance(input_value)
