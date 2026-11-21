from pydantic import JsonValue

from azure_realtime import run as realtime_run
from azure_realtime.types import Input as RealtimeInput

from .contracts import FIELDS, REALTIME_IN, RealtimeCase


def realtime_input(case: RealtimeCase, kind: str) -> RealtimeInput:
    payload: dict[str, JsonValue] = {
        "kind": kind,
        "model": case["model"],
        "scope_headers": case["scope_headers"],
        "ssl_verification": case["ssl_verification"],
        "max_message_size_bytes": case["max_message_size_bytes"],
        "azure_api_base": case["azure_api_base"],
        "azure_api_key": case["azure_api_key"],
        "azure_api_version": case["azure_api_version"],
        "queries": case["queries"],
        "protocols": case["protocols"],
        "failures": case["failures"],
        "http_setups": case["http_setups"],
    }
    return REALTIME_IN.validate_python(payload)


def realtime(case: RealtimeCase) -> JsonValue:
    connects = FIELDS.validate_python(realtime_run(realtime_input(case, "connect")))
    http = FIELDS.validate_python(realtime_run(realtime_input(case, "http")))
    return {"connects": connects["connects"], "http": http["http"]}
