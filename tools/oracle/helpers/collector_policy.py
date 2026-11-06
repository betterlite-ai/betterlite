import os
from typing import cast
from unittest import mock

from litellm._logging import verbose_proxy_logger
from litellm.proxy.spend_tracking.spend_event_producer import (
    CollectorSettings,
    TcpAddress,
    UnixAddress,
    parse_collector_address,
)
from litellm.proxy.spend_tracking.spend_log_error_logger import should_suppress_spend_log_tracebacks

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    addresses: list[Json] = []
    for raw in cast(list[str], input_value["addresses"]):
        try:
            address = parse_collector_address(raw)
        except ValueError:
            addresses.append({"kind": "syntax"})
            continue
        if isinstance(address, UnixAddress):
            addresses.append({"kind": "unix", "path": address.path})
        elif isinstance(address, TcpAddress):
            addresses.append({"kind": "tcp", "host": address.host, "port": address.port})
        else:
            addresses.append({"kind": "invalid"})
    with mock.patch.dict(os.environ, {}, clear=True):
        produces: list[Json] = [
            CollectorSettings.model_validate(settings).produces
            for settings in cast(list[dict[str, Json]], input_value.get("settings", []))
        ]
    suppress: list[Json] = []
    for policy in cast(list[dict[str, Json]], input_value["policies"]):
        value = cast(str | None, policy.get("value"))
        env = {} if value is None else {"LITELLM_SUPPRESS_SPEND_LOG_TRACEBACKS": value}
        with (
            mock.patch.dict(os.environ, env, clear=True),
            mock.patch.object(verbose_proxy_logger, "isEnabledFor", return_value=policy["debug"]),
        ):
            suppress.append(should_suppress_spend_log_tracebacks())
    return {"addresses": addresses, "produces": produces, "suppress": suppress}
