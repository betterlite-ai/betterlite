from datetime import UTC, datetime
from typing import Final

import litellm
from pydantic import JsonValue, TypeAdapter

from .inputs import (
    PROVIDER_METRIC,
    CustomerCall,
    CustomerInput,
    CustomerRowCall,
    EntityInput,
    Input,
    KeyCall,
    KeyListCall,
    OrgCall,
    OrgListCall,
    ProviderCall,
    RemainingBudgetCall,
    RemainingHoursCall,
    TeamCall,
    TeamMembersCall,
    UserCall,
)
from .metrics import ProbeLogger, SampleRow

_JSON: Final = TypeAdapter(JsonValue)


def sample_output(logger: ProbeLogger) -> JsonValue:
    payload: dict[str, list[SampleRow]] = {"samples": logger.samples()}
    return _JSON.validate_python(payload)


def remaining_budget(call: RemainingBudgetCall) -> JsonValue:
    logger = ProbeLogger({}, datetime(1970, 1, 1, tzinfo=UTC))
    payload: dict[str, str] = {"remaining": format(logger.remaining_budget(call), ".6f")}
    return _JSON.validate_python(payload)


def remaining_hours(call: RemainingHoursCall) -> JsonValue:
    logger = ProbeLogger({}, datetime.fromisoformat(call["now"]))
    payload: dict[str, str] = {"hours": format(logger.remaining_hours(call), ".6f")}
    return _JSON.validate_python(payload)


def team(call: TeamCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.entity(call)
    return sample_output(logger)


def org(call: OrgCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.entity(call)
    return sample_output(logger)


def key(call: KeyCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.entity(call)
    return sample_output(logger)


def user(call: UserCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.entity(call)
    return sample_output(logger)


def customer(call: CustomerCall) -> JsonValue:
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.customer(call)
    return sample_output(logger)


def customer_row(call: CustomerRowCall) -> JsonValue:
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.customer_row(call)
    return sample_output(logger)


def team_members(call: TeamMembersCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime(1970, 1, 1, tzinfo=UTC))
    logger.team_members(call)
    return sample_output(logger)


def provider(call: ProviderCall) -> JsonValue:
    logger = ProbeLogger({PROVIDER_METRIC: ["api_provider"]}, datetime(1970, 1, 1, tzinfo=UTC))
    logger.provider(call)
    return sample_output(logger)


def key_list(call: KeyListCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.key_list(call)
    return sample_output(logger)


def org_list(call: OrgListCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime.fromisoformat(call["now"]))
    logger.org_list(call)
    return sample_output(logger)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "remaining_budget":
        return remaining_budget(input_value)
    if input_value["function"] == "remaining_hours":
        return remaining_hours(input_value)
    return entity_dispatch(input_value)


def entity_dispatch(input_value: EntityInput) -> JsonValue:
    if input_value["function"] == "team":
        return team(input_value)
    if input_value["function"] == "org":
        return org(input_value)
    if input_value["function"] == "key":
        return key(input_value)
    if input_value["function"] == "user":
        return user(input_value)
    return customer_dispatch(input_value)


def customer_dispatch(input_value: CustomerInput) -> JsonValue:
    if input_value["function"] == "customer":
        answer = customer(input_value)
    elif input_value["function"] == "team_members":
        answer = team_members(input_value)
    elif input_value["function"] == "provider":
        answer = provider(input_value)
    elif input_value["function"] == "key_list":
        answer = key_list(input_value)
    elif input_value["function"] == "org_list":
        answer = org_list(input_value)
    else:
        answer = customer_row(input_value)
    return answer
