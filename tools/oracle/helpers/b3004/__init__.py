import asyncio
from typing import Final

import litellm
from litellm.exceptions import BudgetExceededError
from litellm.proxy import proxy_server
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.spend_tracking import budget_reservation
from pydantic import ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

from .fakes import InputCost, InputTokens, MaxCost, Pipeline, ScriptedMemory, ScriptedSources
from .models import BudgetReservation, Input
from .views import (
    error_json,
    reservation_json,
    token_object,
    user_object,
)

ROUTE: Final[str] = "/v1/chat/completions"
REQUEST_BODY: Final[dict[str, JsonValue]] = {"model": "gpt-4o"}
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY: Final[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)
# the LiteLLM module the case drives, read through the protocol its call is typed against
ENGINE: Final[BudgetReservation] = TypeAdapter(
    BudgetReservation, config=ARBITRARY
).validate_python(budget_reservation)
APPLY_USER_BUDGET_TO_TEAM_KEYS: Final[bool] = False


async def execute(data: Input) -> dict[str, JsonValue]:
    values = data.get("memory")
    if values is None:
        values = {}
    failures = data.get("failures")
    if failures is None:
        failures = []
    sources = data.get("sources")
    if sources is None:
        sources = {}
    pipeline_fails = data.get("pipeline_fails")
    if pipeline_fails is None:
        pipeline_fails = False
    token = token_object(data["token"])
    user = user_object(data.get("user"))
    memory = ScriptedMemory(values, frozenset(failures))
    source_cache = UserApiKeyCache(in_memory_cache=ScriptedSources(sources))
    pipeline = Pipeline(fail=pipeline_fails)
    # each helper call runs in its own process, so the proxy globals get set in place
    proxy_server.spend_counter_cache.in_memory_cache = memory
    proxy_server.spend_counter_cache.redis_cache = None
    proxy_server.prisma_client = None
    proxy_server.user_api_key_cache = source_cache
    settings: dict[str, JsonValue] = {}
    proxy_server.general_settings = settings
    litellm.budget_exceeded_throttle_percentage = data["throttle_percentage"]
    budget_reservation.estimate_request_max_cost = MaxCost(data["reservation_cost"])
    budget_reservation.estimate_request_input_cost = InputCost()
    budget_reservation.count_request_input_tokens = InputTokens()
    proxy_server.run_spend_counter_pipeline = pipeline
    reservation: dict[str, JsonValue] | None = None
    failure: JsonValue = None
    try:
        reservation = await ENGINE.reserve_budget_for_request(
            REQUEST_BODY,
            ROUTE,
            None,
            token,
            None,
            user,
            None,
            source_cache,
            proxy_server.proxy_logging_obj,
            None,
            None,
            APPLY_USER_BUDGET_TO_TEAM_KEYS,
            data["fail_closed"],
        )
    except (BudgetExceededError, HTTPException) as error:
        failure = error_json(error)
    output: dict[str, JsonValue] = {
        "reservation": reservation_json(reservation),
        "error": failure,
        "memory": {**memory.values},
        "pipelines": pipeline.calls,
        "invalidated": [*memory.deleted],
    }
    return output


def run(input_value: Input) -> JsonValue:
    output = asyncio.run(execute(input_value))
    return JSON.validate_python(output)
