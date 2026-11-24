from collections.abc import Mapping
from typing import Final

from betterlite import _native
from betterlite._endpoints.payload import CallValue, to_call_payload

# ast-grep-ignore: no-long-names
DEFAULT_PASS_THROUGH_REQUEST_TIMEOUT_SECONDS: Final = 600.0


# ast-grep-ignore: no-long-names, no-one-line-functions
def resolve_pass_through_request_timeout(endpoint_timeout: float | None = None) -> float:
    return _native.resolve_passthrough_timeout(endpoint_timeout)


# ast-grep-ignore: no-long-names
def resolve_llm_passthrough_timeout(
    kwargs: Mapping[str, CallValue] | None = None,
    litellm_params: Mapping[str, CallValue] | None = None,
    router_timeout: float | str | None = None,
    router_stream_timeout: float | str | None = None,
) -> float:
    request: Mapping[str, CallValue] = {} if kwargs is None else kwargs
    deployment: Mapping[str, CallValue] = {} if litellm_params is None else litellm_params
    payload = to_call_payload({
        "kwargs": {
            "stream": bool(request.get("stream")),
            "stream_timeout": request.get("stream_timeout"),
            "timeout": request.get("timeout"),
            "request_timeout": request.get("request_timeout"),
        },
        "litellm_params": {
            "stream_timeout": deployment.get("stream_timeout"),
            "timeout": deployment.get("timeout"),
            "request_timeout": deployment.get("request_timeout"),
        },
        "router_timeout": router_timeout,
        "router_stream_timeout": router_stream_timeout,
    })
    return _native.resolve_sdk_timeout(payload.text)
