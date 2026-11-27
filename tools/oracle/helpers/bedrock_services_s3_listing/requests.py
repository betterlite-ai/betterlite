import json
import xml.etree.ElementTree as ET
from typing import Final

from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from litellm.types.llms.openai import FileContentRequest
from pydantic import JsonValue

from .wire import (
    JSON,
    FileCall,
    ListCall,
    NextCall,
    optional_of,
    params_of,
    reply,
    reset_environment,
    response_of,
    visible,
)


def list_request(config: BedrockFilesConfig, call: ListCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    try:
        url, query = config.transform_list_files_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1309-L1317
            call.get("purpose"),
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1309-L1317
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    query_value: Final = JSON.validate_json(json.dumps(query))
    return reply(
        {
            "failed": False,
            "url": url,
            "query": query_value,
            "litellm_params": visible(params),
        }
    )


def next_request(config: BedrockFilesConfig, call: NextCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    response = response_of(call["status"], call["body"], call.get("headers"))
    try:
        signed = config.transform_list_files_next_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1319-L1335
            response,
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1319-L1335
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except (ValueError, ET.ParseError):
        return reply({"failed": True, "status": None})
    if signed is None:
        return reply({"failed": False, "next": None})
    url, query = signed
    query_value: Final = JSON.validate_json(json.dumps(query))
    next_value: dict[str, JsonValue] = {
        "url": url,
        "query": query_value,
        "litellm_params": visible(params),
    }
    return reply({"failed": False, "next": next_value})


def delete_request(config: BedrockFilesConfig, call: FileCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    declared = call.get("file_id")
    file_id = "" if declared is None else declared
    try:
        url, query = config.transform_delete_file_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1284-L1292
            file_id,
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1284-L1292
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    query_value: Final = JSON.validate_json(json.dumps(query))
    return reply(
        {
            "failed": False,
            "url": url,
            "query": query_value,
            "litellm_params": visible(params),
        }
    )


def content_request(config: BedrockFilesConfig, call: FileCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    request: FileContentRequest = {}
    declared = call.get("file_id")
    if declared is not None:
        request["file_id"] = declared
    try:
        url, query = config.transform_file_content_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1393-L1412
            request,
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1393-L1412
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    query_value: Final = JSON.validate_json(json.dumps(query))
    return reply(
        {
            "failed": False,
            "url": url,
            "query": query_value,
            "litellm_params": visible(params),
        }
    )
