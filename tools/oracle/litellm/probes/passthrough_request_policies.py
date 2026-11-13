import sys
from collections.abc import Generator
from contextlib import contextmanager
from types import ModuleType
from typing import Final, Literal, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.constants import DEFAULT_REQUEST_TIMEOUT_SECONDS
from litellm.passthrough.timeout_utils import (
    resolve_llm_passthrough_timeout,
    resolve_pass_through_request_timeout,
)
from litellm.passthrough.utils import BasePassthroughUtils
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
TimeoutInput = float | int | str | None


class RequestInput(Strict):
    stream: bool = False
    stream_timeout: TimeoutInput = None
    timeout: TimeoutInput = None
    request_timeout: TimeoutInput = None


class DeploymentInput(Strict):
    stream_timeout: TimeoutInput = None
    timeout: TimeoutInput = None
    request_timeout: TimeoutInput = None


class RouterInput(Strict):
    stream_timeout: TimeoutInput = None
    timeout: TimeoutInput = None


class Output(Strict):
    query: dict[str, str | list[str]] | None = None
    headers: dict[str, str] | None = None
    seconds: float | None = None


def settings_mapping(settings: float | None) -> dict[str, float]:
    if settings is None:
        return {}
    return {"pass_through_request_timeout": settings}


@contextmanager
def proxy_module(settings: float | None) -> Generator[None, None, None]:
    module = ModuleType(PROXY_MODULE)
    module.general_settings = settings_mapping(settings)  # pyright: ignore[reportAttributeAccessIssue]
    previous = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = module
    try:
        yield
    finally:
        if previous is None:
            _ = sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = previous


class PassthroughRequestPolicies(Operation):
    mode: Literal["query_merge", "forward_headers", "proxy_timeout", "sdk_timeout"]
    existing_url: str | None = None
    request_query_params: dict[str, str | list[str]] | None = None
    default_query_params: dict[str, str | list[str]] | None = None
    request_headers: dict[str, str] | None = None
    custom_headers: dict[str, str] | None = None
    forward_headers: bool | None = None
    endpoint_timeout: float | None = None
    settings_timeout: float | None = None
    request: RequestInput = Field(default_factory=RequestInput)
    deployment: DeploymentInput = Field(default_factory=DeploymentInput)
    router: RouterInput = Field(default_factory=RouterInput)
    configured_timeout: float | None = None

    @override
    def run(self) -> Strict:
        if self.mode == "query_merge":
            return Output(query=self.merged_query())
        if self.mode == "forward_headers":
            return Output(headers=self.forwarded_headers())
        if self.mode == "proxy_timeout":
            return Output(seconds=self.generic_timeout())
        return Output(seconds=self.sdk_timeout())

    def merged_query(self) -> dict[str, str | list[str]]:
        if self.existing_url is None:
            raise ProbeError("invalid_request", "query_merge needs existing_url")
        request = {} if self.request_query_params is None else self.request_query_params
        return BasePassthroughUtils.get_merged_query_parameters(  # pyright: ignore[reportUnknownMemberType]
            existing_url=httpx.URL(self.existing_url),
            request_query_params=request,
            default_query_params=self.default_query_params,
        )

    def forwarded_headers(self) -> dict[str, str]:
        merged = BasePassthroughUtils.forward_headers_from_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            request_headers={} if self.request_headers is None else self.request_headers,
            headers={} if self.custom_headers is None else self.custom_headers,
            forward_headers=self.forward_headers,
        )
        return cast("dict[str, str]", merged)

    def generic_timeout(self) -> float:
        with proxy_module(self.settings_timeout):
            return resolve_pass_through_request_timeout(endpoint_timeout=self.endpoint_timeout)

    def sdk_timeout(self) -> float:
        kwargs: dict[str, object] = {"stream": self.request.stream}
        if self.request.stream_timeout is not None:
            kwargs["stream_timeout"] = self.request.stream_timeout
        if self.request.timeout is not None:
            kwargs["timeout"] = self.request.timeout
        if self.request.request_timeout is not None:
            kwargs["request_timeout"] = self.request.request_timeout
        params: dict[str, object] = {}
        if self.deployment.stream_timeout is not None:
            params["stream_timeout"] = self.deployment.stream_timeout
        if self.deployment.timeout is not None:
            params["timeout"] = self.deployment.timeout
        if self.deployment.request_timeout is not None:
            params["request_timeout"] = self.deployment.request_timeout
        configured = self.configured_timeout
        request_timeout = DEFAULT_REQUEST_TIMEOUT_SECONDS if configured is None else configured
        with (
            proxy_module(self.settings_timeout),
            patch.object(litellm, "request_timeout", request_timeout),
            patch.object(litellm, "request_timeout_explicitly_set", configured is not None),
        ):
            return resolve_llm_passthrough_timeout(
                kwargs=kwargs,
                litellm_params=params,
                router_timeout=self.router.timeout,
                router_stream_timeout=self.router.stream_timeout,
            )
