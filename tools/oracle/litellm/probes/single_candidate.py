from typing import Annotated

from litellm.router_strategy.simple_shuffle import simple_shuffle
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false


class Deployment(Strict):
    id: str
    weight: Annotated[int, Field(ge=0)] = 0
    rpm: Annotated[int, Field(ge=0)] = 0
    tpm: Annotated[int, Field(ge=0)] = 0


class Output(Strict):
    selected: str


class SingleCandidate(Operation):
    deployments: Annotated[list[Deployment], Field(min_length=1)]

    @override
    def run(self) -> Output:
        deployments: list[dict[str, object]] = [
            {
                "litellm_params": {"weight": deployment.weight, "rpm": deployment.rpm, "tpm": deployment.tpm},
                "model_info": {"id": deployment.id},
            }
            for deployment in self.deployments
        ]
        selected = simple_shuffle(lambda _model: None, deployments, "probe", None)
        index = next(index for index, deployment in enumerate(deployments) if deployment is selected)
        return Output(selected=self.deployments[index].id)
