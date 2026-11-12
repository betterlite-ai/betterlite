import json
import logging
from collections.abc import Mapping
from pathlib import PurePosixPath
from typing import Final, Literal, TypedDict, cast

from litellm.proxy._types import ProxyException
from litellm.proxy.batches_endpoints.common_utils import validate_batch_list_limit
from litellm.router_utils.batch_utils import (
    InMemoryFile,
    _get_router_metadata_variable_name,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L153
    is_batch_retrieve_call_type,
    parse_jsonl_with_embedded_newlines,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L18
    replace_model_in_jsonl,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L76
    should_replace_model_in_jsonl,
)
from litellm.types.llms.openai import OpenAIFilesPurpose
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
LOGGER: Final = logging.getLogger("LiteLLM")


class Utf8File(TypedDict):
    kind: Literal["utf8"]
    text: str


class HexFile(TypedDict):
    kind: Literal["hex"]
    hex: str


class PathFile(TypedDict):
    kind: Literal["path"]
    path: str


class ParseInput(TypedDict):
    function: Literal["parse_jsonl"]
    content: str


class ReplaceInput(TypedDict):
    function: Literal["replace_model"]
    file: Utf8File | HexFile | PathFile
    new_model_name: str


class ShouldReplaceInput(TypedDict):
    function: Literal["should_replace_model"]
    purpose: OpenAIFilesPurpose
    passthrough: bool


class MetadataNameInput(TypedDict):
    function: Literal["router_metadata_name"]
    function_name: str | None


class RetrieveCallTypeInput(TypedDict):
    function: Literal["is_batch_retrieve_call_type"]
    call_type: str


class ListLimitInput(TypedDict):
    function: Literal["validate_list_limit"]
    limit: int | None


Input = (
    ParseInput
    | ReplaceInput
    | ShouldReplaceInput
    | MetadataNameInput
    | RetrieveCallTypeInput
    | ListLimitInput
)


class PreviewCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.previews: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        match record.args:
            case tuple() as args:
                self.previews.append(str(args[0]))
            case Mapping() | None:
                return


def parse(fields: ParseInput) -> JsonValue:
    capture = PreviewCapture()
    LOGGER.addHandler(capture)
    try:
        rows = JSON.validate_python(parse_jsonl_with_embedded_newlines(fields["content"]))  # type: ignore[misc]  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L18
    except json.JSONDecodeError:
        return {"rows": None, "logged_preview": capture.previews[0]}
    finally:
        LOGGER.removeHandler(capture)
    return {"rows": rows, "logged_preview": None}


def replace(fields: ReplaceInput) -> JsonValue:
    file = fields["file"]
    source: PurePosixPath | bytes
    if file["kind"] == "path":
        source = PurePosixPath(file["path"])
    elif file["kind"] == "hex":
        source = bytes.fromhex(file["hex"])
    else:
        source = file["text"].encode("utf-8")
    capture = PreviewCapture()
    LOGGER.addHandler(capture)
    try:
        replaced = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L76
            "InMemoryFile | PurePosixPath | bytes",
            replace_model_in_jsonl(source, fields["new_model_name"]),  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L76
        )
    finally:
        LOGGER.removeHandler(capture)
    logged = capture.previews[0] if capture.previews else None
    match replaced:
        case PurePosixPath() | bytes():
            return {"kind": "original", "logged_preview": logged}
        case InMemoryFile():
            lines = replaced.getvalue().decode("utf-8").split("\n")
            return {
                "kind": "rewritten",
                "rows": [JSON.validate_json(line) for line in lines],
                "name": str(replaced.name),  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/batch_utils.py#L13
                "content_type": replaced.content_type,
                "logged_preview": logged,
            }


def list_limit(fields: ListLimitInput) -> JsonValue:
    try:
        validate_batch_list_limit(fields["limit"])
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {
            "accepted": False,
            "message": error.message,
            "type": error.type,
            "param": error.param,
            "code": error.code,
            "openai_code": error.openai_code,
            "headers": JSON.validate_python(error.headers),
        }
    return {"accepted": True}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "parse_jsonl":
        return parse(input_value)
    if input_value["function"] == "replace_model":
        return replace(input_value)
    if input_value["function"] == "should_replace_model":
        return should_replace_model_in_jsonl(input_value["purpose"], input_value["passthrough"])
    if input_value["function"] == "router_metadata_name":
        return _get_router_metadata_variable_name(input_value["function_name"])
    if input_value["function"] == "is_batch_retrieve_call_type":
        return is_batch_retrieve_call_type(input_value["call_type"])
    return list_limit(input_value)
