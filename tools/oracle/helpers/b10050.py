import sys
from types import ModuleType
from typing import Final, TypedDict

import litellm
from litellm.proxy.auth.auth_utils import get_end_user_id_from_request_body
from pydantic import JsonValue

PROXY_MODULE: Final = "litellm.proxy.proxy_server"


class Mapping(TypedDict):
    header_name: str
    litellm_user_role: str


class RequestInput(TypedDict):
    body: dict[str, JsonValue]
    headers: list[tuple[str, str]] | None


class Input(TypedDict):
    user_header_mappings: list[Mapping]
    user_header_name: str | None
    validate_end_user_id_in_db: bool
    requests: list[RequestInput]


class ProxyServer(ModuleType):
    general_settings: dict[str, JsonValue]


# a case sends the same mapping shape the config loader produces, so only the name survives
def mapping_value(mapping: Mapping) -> dict[str, JsonValue]:
    return {
        "header_name": mapping["header_name"],
        "litellm_user_role": mapping["litellm_user_role"],
    }


def run(input_value: Input) -> JsonValue:
    mappings: list[JsonValue] = [
        mapping_value(mapping) for mapping in input_value["user_header_mappings"]
    ]
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.general_settings = {
        "user_header_mappings": mappings,
        "user_header_name": input_value["user_header_name"],
    }
    # get_end_user_id_from_request_body imports general_settings from this module at call time
    sys.modules[PROXY_MODULE] = proxy_server
    litellm.validate_end_user_id_in_db = input_value["validate_end_user_id_in_db"]
    ids: list[JsonValue] = []
    for request in input_value["requests"]:
        headers = request["headers"]
        request_headers = None if headers is None else dict(headers)
        ids.append(get_end_user_id_from_request_body(request["body"], request_headers))
    return {"ids": ids}
