from typing import Annotated, Literal

from litellm.exceptions import BadRequestError
from litellm.router import Router
from litellm.types.router import Deployment, LiteLLM_Params, ModelInfo
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnknownMemberType=false


API_KEY = "sk-probe"


class Entry(Strict):
    model_name: str
    model: str
    id: str


class SetList(Strict):
    action: Literal["set"]
    models: list[Entry]


class Add(Strict):
    action: Literal["add"]
    model: Entry


class Upsert(Strict):
    action: Literal["upsert"]
    model: Entry


class Delete(Strict):
    action: Literal["delete"]
    id: str


class Observed(Strict):
    raised: bool
    returned: bool
    names: list[str]
    ids: list[str]
    by_name: dict[str, list[str]]


class Output(Strict):
    steps: list[Observed]


def deployment(entry: Entry) -> Deployment:
    return Deployment(
        model_name=entry.model_name,
        litellm_params=LiteLLM_Params(model=entry.model, api_key=API_KEY),
        model_info=ModelInfo(id=entry.id),
    )


# The model list in order, the id index, and the ids each model name indexes, as the router's public readers show them.
def observe(router: Router, *, raised: bool, returned: Deployment | None) -> Observed:
    names = [listed["model_name"] for listed in router.get_model_list() or []]
    return Observed(
        raised=raised,
        returned=returned is not None,
        names=names,
        ids=sorted(router.get_model_ids()),
        by_name={name: router.get_model_ids(name) for name in names},
    )


class CatalogUpdates(Operation):
    ignore_invalid_deployments: bool
    steps: list[Annotated[SetList | Add | Upsert | Delete, Field(discriminator="action")]]

    @override
    def run(self) -> Output:
        router = Router(model_list=[], ignore_invalid_deployments=self.ignore_invalid_deployments)
        observed: list[Observed] = []
        for step in self.steps:
            returned: Deployment | None = None
            raised = False
            try:
                if isinstance(step, SetList):
                    router.set_model_list(
                        [
                            {
                                "model_name": entry.model_name,
                                "litellm_params": {"model": entry.model, "api_key": API_KEY},
                                "model_info": {"id": entry.id},
                            }
                            for entry in step.models
                        ]
                    )
                elif isinstance(step, Add):
                    returned = router.add_deployment(deployment(step.model))
                elif isinstance(step, Upsert):
                    returned = router.upsert_deployment(deployment(step.model))
                else:
                    returned = router.delete_deployment(step.id)
            except (ValueError, TypeError, BadRequestError):
                raised = True
            observed.append(observe(router, raised=raised, returned=returned))
        return Output(steps=observed)
