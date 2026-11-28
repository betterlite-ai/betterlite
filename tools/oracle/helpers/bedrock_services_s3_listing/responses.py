import json
import xml.etree.ElementTree as ET
from typing import Final, TypedDict, cast
from unittest.mock import MagicMock

import httpx
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from pydantic import JsonValue, TypeAdapter

from .calls import (
    JSON,
    DeleteResponseCall,
    ListingCall,
    RetrieveCall,
    params_of,
    reply,
    reset_environment,
    response_of,
)


class FileReply(TypedDict):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: str
    status: str | None


class FileDeletedReply(TypedDict):
    id: str
    object: str
    deleted: bool


def list_response(config: BedrockFilesConfig, call: ListingCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    response = response_of(call["status"], call["body"], call.get("headers"))
    try:
        files = config.transform_list_files_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1363-L1391
            response, MagicMock(), params
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except (ValueError, ET.ParseError):
        return reply({"failed": True, "status": None})
    listed: Final = [TypeAdapter(FileReply).validate_json(item.model_dump_json()) for item in files]
    return reply({"failed": False, "files": JSON.validate_json(json.dumps(listed))})


def delete_response(config: BedrockFilesConfig, call: DeleteResponseCall) -> JsonValue:
    response = response_of(call["status"], call["body"], call.get("headers"))
    declared = call.get("additional_args")
    details: dict[str, JsonValue] = {}
    if declared is not None:
        details["additional_args"] = declared
    logging_obj = MagicMock()
    logging_obj.model_call_details = details
    try:
        deleted = config.transform_delete_file_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1294-L1307
            response, logging_obj, {}
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    deleted_reply: Final = TypeAdapter(FileDeletedReply).validate_json(deleted.model_dump_json())
    return reply({"failed": False, "file": JSON.validate_json(json.dumps(deleted_reply))})


def retrieve(config: BedrockFilesConfig, call: RetrieveCall) -> list[JsonValue]:
    params: dict[str, JsonValue] = {}
    optional: dict[str, JsonValue] = {}
    response = httpx.Response(status_code=200, content=b"")
    outcomes: list[JsonValue] = []
    try:
        _request = cast("tuple[str, dict[str, str]]", config.transform_retrieve_file_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1268-L1274
            call["file_id"], optional, params
        ))
    except NotImplementedError:
        outcomes.append(reply({"failed": True, "status": None}))
    try:
        _response = config.transform_retrieve_file_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1276-L1282
            response, MagicMock(), params
        )
    except NotImplementedError:
        outcomes.append(reply({"failed": True, "status": None}))
    return outcomes
