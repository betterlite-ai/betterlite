from dataclasses import dataclass

import litellm
from pydantic import JsonValue as Json

from .probe import DOCUMENT

litellm.suppress_debug_info = True


@dataclass
class Record:
    input: Json
    additional_args: dict[str, Json] | None
    original_response: Json = None


def arguments(kwargs: dict[str, Json]) -> dict[str, Json] | None:
    additional = kwargs.get("additional_args")
    match additional:
        case None:
            return None
        case dict() as fields:
            return DOCUMENT.validate_python(fields)
        case bool() | int() | float() | str() | list():
            raise TypeError("additional_args must be an object")


class Recorder:
    def __init__(self) -> None:
        self.pre_calls: list[Record] = []
        self.post_calls: list[Record] = []

    def pre_call(self, **kwargs: Json) -> None:
        record = Record(input=kwargs["input"], additional_args=arguments(kwargs))
        self.pre_calls.append(record)

    def post_call(self, **kwargs: Json) -> None:
        record = Record(
            input=kwargs["input"],
            additional_args=arguments(kwargs),
            original_response=response_value(kwargs["original_response"]),
        )
        self.post_calls.append(record)


def arguments_value(args: dict[str, Json] | None) -> Json:
    if args is None:
        return {
            "api_base": None,
            "complete_input_dict": None,
            "acompletion": None,
            "header_names": [],
        }
    base: Json = None
    match args.get("api_base"):
        case str() as api_base:
            base = api_base.rstrip("/")
        case None | bool() | int() | float() | list() | dict():
            base = None
    names: list[Json] = []
    match args.get("headers"):
        case dict() as fields:
            typed = DOCUMENT.validate_python(fields)
            names.extend(sorted(typed))
        case None | bool() | int() | float() | str() | list():
            names = []
    return {
        "api_base": base,
        "complete_input_dict": args.get("complete_input_dict"),
        "acompletion": args.get("acompletion"),
        "header_names": names,
    }


def response_value(original: Json) -> Json:
    match original:
        case dict() as found:
            return DOCUMENT.validate_python(found)
        case None | bool() | int() | float() | str() | list():
            return None


def record_value(record: Record) -> Json:
    additional = arguments_value(record.additional_args)
    return {
        "input": record.input,
        "additional_args": additional,
        "original_response": record.original_response,
    }


def last(records: list[Record]) -> Json:
    if not records:
        return None
    return record_value(records[-1])
