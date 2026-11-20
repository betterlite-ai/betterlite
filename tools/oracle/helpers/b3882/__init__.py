from datetime import UTC, datetime
from typing import Final

import litellm
from pydantic import JsonValue, TypeAdapter

from .inputs import (
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
from .metrics import ProbeLogger

_JSON: Final = TypeAdapter(JsonValue)

# LiteLLM fixes this one gauge's label list to api_provider, so its cases name no labels
PROVIDER_METRIC: Final = "litellm_provider_remaining_budget_metric"


def probe(labels_by_metric: dict[str, list[str]], now: str) -> ProbeLogger:
    return ProbeLogger(labels_by_metric, datetime.fromisoformat(now))


def sample_output(logger: ProbeLogger) -> JsonValue:
    return _JSON.validate_python({"samples": logger.samples()})


def remaining_budget(call: RemainingBudgetCall) -> JsonValue:
    logger = ProbeLogger({}, datetime(1970, 1, 1, tzinfo=UTC))
    remaining = logger.remaining_budget(call)
    return _JSON.validate_python({"remaining": format(remaining, ".6f")})


def remaining_hours(call: RemainingHoursCall) -> JsonValue:
    logger = ProbeLogger({}, datetime.fromisoformat(call["now"]))
    hours = logger.remaining_hours(call)
    return _JSON.validate_python({"hours": format(hours, ".6f")})


def team(call: TeamCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.team(call)
    return sample_output(logger)


def org(call: OrgCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.org(call)
    return sample_output(logger)


def key(call: KeyCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.key(call)
    return sample_output(logger)


def user(call: UserCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.user(call)
    return sample_output(logger)


def customer(call: CustomerCall) -> JsonValue:
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    logger = probe(call["labels_by_metric"], call["now"])
    logger.customer(call)
    return sample_output(logger)


def customer_row(call: CustomerRowCall) -> JsonValue:
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    logger = probe(call["labels_by_metric"], call["now"])
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
    logger = probe(call["labels_by_metric"], call["now"])
    logger.key_list(call)
    return sample_output(logger)


def org_list(call: OrgListCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
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
        return customer(input_value)
    if input_value["function"] == "team_members":
        return team_members(input_value)
    if input_value["function"] == "provider":
        return provider(input_value)
    if input_value["function"] == "key_list":
        return key_list(input_value)
    if input_value["function"] == "org_list":
        return org_list(input_value)
    return customer_row(input_value)
