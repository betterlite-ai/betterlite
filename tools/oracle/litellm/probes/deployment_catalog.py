from typing import ClassVar, Literal, Protocol

from litellm.exceptions import BadRequestError
from litellm.router import Router
from litellm.types.router import RouterRateLimitError
from pydantic import BaseModel, ConfigDict, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnknownMemberType=false


API_KEY = "sk-probe"


class Entry(Strict):
    model_name: str
    model: str
    id: str
    team_id: str | None = None
    team_public_model_name: str | None = None


class Lookup(Strict):
    model: str
    specific_deployment: bool
    team_id: str | None = None


class Selected(Strict):
    model_name: str
    model: str
    id: str


class Found(Strict):
    outcome: Literal["selected", "bad_request", "empty_pool"]
    selected: Selected | None


class Output(Strict):
    lookups: list[Found]


class Selector(Protocol):
    def get_available_deployment(
        self, model: str, *, specific_deployment: bool, request_kwargs: dict[str, object] | None
    ) -> object: ...


class Params(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model: str


class Info(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    id: str


class Returned(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_name: str
    litellm_params: Params
    model_info: Info


def model_list_entry(entry: Entry) -> dict[str, object]:
    info = {"id": entry.id}
    if entry.team_id is not None:
        info["team_id"] = entry.team_id
    if entry.team_public_model_name is not None:
        info["team_public_model_name"] = entry.team_public_model_name
    return {
        "model_name": entry.model_name,
        "litellm_params": {"model": entry.model, "api_key": API_KEY},
        "model_info": info,
    }


def answer(selector: Selector, lookup: Lookup) -> Found:
    request_kwargs: dict[str, object] | None = (
        None if lookup.team_id is None else {"metadata": {"user_api_key_team_id": lookup.team_id}}
    )
    try:
        deployment = selector.get_available_deployment(
            model=lookup.model, specific_deployment=lookup.specific_deployment, request_kwargs=request_kwargs
        )
    except BadRequestError:
        return Found(outcome="bad_request", selected=None)
    except RouterRateLimitError:
        return Found(outcome="empty_pool", selected=None)
    returned = TypeAdapter(Returned).validate_python(deployment)
    selected = Selected(model_name=returned.model_name, model=returned.litellm_params.model, id=returned.model_info.id)
    return Found(outcome="selected", selected=selected)


class DeploymentCatalog(Operation):
    models: list[Entry]
    model_group_alias: dict[str, str]
    lookups: list[Lookup]

    @override
    def run(self) -> Output:
        router = Router(
            model_list=[model_list_entry(entry) for entry in self.models],
            model_group_alias=dict(self.model_group_alias),
        )
        return Output(lookups=[answer(router, lookup) for lookup in self.lookups])
