from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils import url_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def validate(input_value: dict[str, Json]) -> Json:
    url = cast(str, input_value["url"])
    allowed_hosts = cast(list[str], input_value.get("user_url_allowed_hosts", []))
    ssl_verify = cast(bool, input_value.get("ssl_verify", True))
    with (
        patch.object(litellm, "user_url_allowed_hosts", allowed_hosts),
        patch.object(litellm, "ssl_verify", ssl_verify),
    ):
        rewritten, host_header = url_utils.validate_url(url)
    return {"url": rewritten, "host_header": host_header}


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("call"):
        case "validate_url":
            return validate(input_value)
        case "blocked_ip":
            addresses = cast(list[str], input_value["addresses"])
            return [
                url_utils._is_blocked_ip(address)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                for address in addresses
            ]
        case other:
            raise ValueError(f"unknown b5082 call: {other!r}")
