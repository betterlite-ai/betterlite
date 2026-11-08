import os
from typing import Literal, TypedDict

from litellm.proxy.middleware.per_request_root_path_middleware import (
    _request_root_path_var,  # pyright: ignore[reportPrivateUsage]
)
from litellm.proxy.utils import get_custom_url, join_paths


class JoinPathsInput(TypedDict):
    function: Literal["join_paths"]
    base_path: str
    route: str


class GetCustomUrlInput(TypedDict):
    function: Literal["get_custom_url"]
    proxy_base_url: str | None
    request_base_url: str
    request_root_path: str
    route: str | None


Input = JoinPathsInput | GetCustomUrlInput


def run(input_value: Input) -> str:
    match input_value["function"]:
        case "join_paths":
            return join_paths(input_value["base_path"], input_value["route"])
        case "get_custom_url":
            proxy_base_url = input_value["proxy_base_url"]
            if proxy_base_url is None:
                os.environ.pop("PROXY_BASE_URL", None)
            else:
                os.environ["PROXY_BASE_URL"] = proxy_base_url
            token = _request_root_path_var.set(input_value["request_root_path"])
            try:
                return get_custom_url(input_value["request_base_url"], input_value["route"])
            finally:
                _request_root_path_var.reset(token)
