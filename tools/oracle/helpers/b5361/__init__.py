import io
from contextlib import redirect_stdout
from typing import NotRequired, TypeAlias, TypedDict

import litellm
from pydantic import TypeAdapter

from b5361.flags import Step, run_step
from b5361.sets import MODULE, SETS, ModelCost, clear

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Fixture(TypedDict):
    model_cost: NotRequired[ModelCost]


class Input(TypedDict):
    fixture: NotRequired[Fixture]
    steps: list[Step]


class Answer(TypedDict):
    sets: dict[str, list[str]]
    by_provider: dict[str, list[str]]
    flags: list[bool]


INPUT = TypeAdapter(Input)


def model_cost_of(fixture: Fixture | None) -> ModelCost:
    if fixture is None:
        return {}
    costs = fixture.get("model_cost")
    return {} if costs is None else costs


def run(input_value: Json) -> Answer:
    parsed = INPUT.validate_python(input_value)
    costs = model_cost_of(parsed.get("fixture"))
    litellm.model_cost = costs
    clear()
    MODULE.add_known_models(costs)
    # the provider lookup prints its provider list to stdout for a model with no provider
    with redirect_stdout(io.StringIO()):
        flags = [run_step(step) for step in parsed["steps"]]
    return {
        "sets": {name: sorted(names) for name, names in SETS.items()},
        "by_provider": {
            provider: sorted(names) for provider, names in MODULE.models_by_provider.items()
        },
        "flags": flags,
    }
