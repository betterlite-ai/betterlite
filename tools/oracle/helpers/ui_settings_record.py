from typing import Final, TypedDict

from litellm.proxy.ui_crud_endpoints.proxy_setting_endpoints import UISettings
from pydantic import JsonValue, TypeAdapter, ValidationError

SETTINGS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


# ValidationError.errors() carries the rejected input as Any, so its JSON form is read instead
class ErrorItem(TypedDict):
    type: str
    loc: list[int | str]


ERRORS: Final[TypeAdapter[list[ErrorItem]]] = TypeAdapter(list[ErrorItem])


class Input(TypedDict):
    settings: dict[str, JsonValue]


class Output(TypedDict):
    accepted: bool
    error_type: str | None
    error_field: int | str | None
    error_index: int | str | None
    dump: dict[str, JsonValue] | None
    exclude_unset: dict[str, JsonValue] | None


def rejected(error: ValidationError) -> Output:
    first = ERRORS.validate_json(error.json())[0]
    loc = first["loc"]
    return {
        "accepted": False,
        "error_type": first["type"],
        "error_field": loc[0],
        "error_index": loc[1] if len(loc) > 1 else None,
        "dump": None,
        "exclude_unset": None,
    }


def run(input_value: Input) -> Output:
    try:
        record = UISettings.model_validate(input_value["settings"])
    except ValidationError as error:
        return rejected(error)
    return {
        "accepted": True,
        "error_type": None,
        "error_field": None,
        "error_index": None,
        "dump": SETTINGS.validate_json(record.model_dump_json()),
        "exclude_unset": SETTINGS.validate_json(record.model_dump_json(exclude_unset=True)),
    }
