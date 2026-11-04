import os
from typing import TypeAlias
from unittest import mock

import litellm
from litellm._logging import verbose_proxy_logger
from litellm.proxy.spend_tracking.spend_event_producer import (
    TcpAddress,
    UnixAddress,
    parse_collector_address,
)
from litellm.proxy.spend_tracking.spend_log_error_logger import should_suppress_spend_log_tracebacks

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    """Compare the collector policy through the pinned LiteLLM behavior."""
    _ = litellm
    address = parse_collector_address(str(input_value["address"]))
    if isinstance(address, UnixAddress):
        rendered: Json = {"kind": "unix", "path": address.path}
    elif isinstance(address, TcpAddress):
        rendered = {"kind": "tcp", "host": address.host, "port": address.port}
    else:
        rendered = {"error": address.reason}
    env_value = input_value.get("suppress")
    env = {} if env_value is None else {"LITELLM_SUPPRESS_SPEND_LOG_TRACEBACKS": str(env_value)}
    debug_enabled = input_value["debug"] is True
    produces = input_value.get("enabled") is True and input_value.get("job_role") != "collector"
    with (
        mock.patch.dict(os.environ, env, clear=False),
        mock.patch.object(verbose_proxy_logger, "isEnabledFor", return_value=debug_enabled),
    ):
        return {
            "address": rendered,
            "produces": produces,
            "suppress": should_suppress_spend_log_tracebacks(),
        }
