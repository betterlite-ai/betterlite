from collections.abc import Sequence
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.litellm_core_utils.bug_report import (
    BugReport,
    EnvironmentReport,
    allowlisted,
    bug_report_notice,
    should_report_bug,
)
from litellm.proxy._types import LiteLLMRoutes
from litellm.proxy.bug_report_config import build_proxy_bug_report

from proxy_endpoint_support import FailureError


class EnvironmentRecord(TypedDict):
    surface: Literal["sdk", "proxy"]
    litellm_version: str
    python_version: str
    deployment: str | None
    config_lines: list[str]


class BugReportRecord(TypedDict):
    environment: EnvironmentRecord
    exception_type: str
    litellm_frames: list[str]
    call_type: str | None
    custom_llm_provider: str | None
    stream: bool | None


class NoticeCall(TypedDict):
    function: Literal["notice"]
    record: BugReportRecord


class FailureCall(TypedDict):
    function: Literal["failure"]
    status_code: int | None


class RoutesCall(TypedDict):
    function: Literal["routes"]
    excluded: list[str]


class RouteCall(TypedDict):
    function: Literal["route"]
    path: str


Call: TypeAlias = NoticeCall | FailureCall | RoutesCall | RouteCall

Answer: TypeAlias = str | BugReportRecord | list[str] | None


class Input(TypedDict):
    env: dict[str, str]
    calls: list[Call]


# KNOWN_PROXY_ROUTES through LiteLLM's members; agent_routes is its two agent tables combined
ROUTE_TABLES: tuple[Sequence[str] | frozenset[str], ...] = (
    LiteLLMRoutes.openai_route_names.value,
    LiteLLMRoutes.openai_routes.value,
    LiteLLMRoutes.mapped_pass_through_routes.value,
    LiteLLMRoutes.passthrough_routes_wildcard.value,
    LiteLLMRoutes.litellm_native_routes.value,
    LiteLLMRoutes.anthropic_routes.value,
    LiteLLMRoutes.mcp_inference_routes.value,
    LiteLLMRoutes.mcp_management_routes.value,
    LiteLLMRoutes.mcp_routes.value,
    LiteLLMRoutes.agent_inference_routes.value,
    LiteLLMRoutes.agent_management_routes.value,
    LiteLLMRoutes.google_routes.value,
    LiteLLMRoutes.apply_guardrail_routes.value,
    LiteLLMRoutes.model_info_routes.value,
    LiteLLMRoutes.llm_api_routes.value,
    LiteLLMRoutes.info_routes.value,
    LiteLLMRoutes.master_key_only_routes.value,
    LiteLLMRoutes.key_management_routes.value,
    LiteLLMRoutes.team_service_account_key_routes.value,
    LiteLLMRoutes.management_routes.value,
    LiteLLMRoutes.spend_tracking_routes.value,
    LiteLLMRoutes.global_spend_tracking_routes.value,
    LiteLLMRoutes.public_routes.value,
    LiteLLMRoutes.ui_routes.value,
    LiteLLMRoutes.compliance_check_routes.value,
    LiteLLMRoutes.internal_user_routes.value,
    LiteLLMRoutes.internal_user_view_only_routes.value,
    LiteLLMRoutes.self_managed_routes.value,
    LiteLLMRoutes.org_admin_only_routes.value,
    LiteLLMRoutes.admin_viewer_routes.value,
    LiteLLMRoutes.org_admin_allowed_routes.value,
)


def report_from_record(record: BugReportRecord) -> BugReport:
    environment = record["environment"]
    return BugReport(
        environment=EnvironmentReport(
            surface=environment["surface"],
            litellm_version=environment["litellm_version"],
            python_version=environment["python_version"],
            deployment=environment["deployment"],
            config_lines=tuple(environment["config_lines"]),
        ),
        exception_type=record["exception_type"],
        litellm_frames=tuple(record["litellm_frames"]),
        call_type=record["call_type"],
        custom_llm_provider=record["custom_llm_provider"],
        stream=record["stream"],
    )


def record_output(report: BugReport) -> BugReportRecord:
    environment = report.environment
    return {
        "environment": {
            "surface": environment.surface,
            "litellm_version": environment.litellm_version,
            "python_version": environment.python_version,
            "deployment": environment.deployment,
            "config_lines": list(environment.config_lines),
        },
        "exception_type": report.exception_type,
        "litellm_frames": list(report.litellm_frames),
        "call_type": report.call_type,
        "custom_llm_provider": report.custom_llm_provider,
        "stream": report.stream,
    }


def notice_answer(call: NoticeCall) -> str:
    report = report_from_record(call["record"])
    return bug_report_notice(report)


def failure_answer(call: FailureCall) -> BugReportRecord | None:
    error = FailureError("boom", call["status_code"], None)
    if not should_report_bug(error):
        return None
    return record_output(build_proxy_bug_report(error))


def known_proxy_routes() -> frozenset[str]:
    return frozenset(
        route for table in ROUTE_TABLES for route in table if route.startswith("/")
    )


def routes_answer(call: RoutesCall) -> list[str]:
    excluded = frozenset(call["excluded"])
    return sorted(route for route in known_proxy_routes() if route not in excluded)


def answer(call: Call) -> Answer:
    match call["function"]:
        case "notice":
            return notice_answer(call)
        case "failure":
            return failure_answer(call)
        case "routes":
            return routes_answer(call)
        case "route":
            return allowlisted(call["path"], known_proxy_routes())


def run(input_value: Input) -> dict[str, list[Answer]]:
    litellm.suppress_debug_info = True
    return {"calls": [answer(call) for call in input_value["calls"]]}
