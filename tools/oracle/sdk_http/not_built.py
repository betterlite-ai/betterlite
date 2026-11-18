from typing import ClassVar

from pydantic import BaseModel, ConfigDict


class NotBuiltError(NotImplementedError):
    def __init__(self, task: str) -> None:
        super().__init__(f"the sdk_http call of task {task} is not built yet")


class Unbuilt(BaseModel):
    model_config = ConfigDict(extra="forbid")

    task: ClassVar[str]

    def serialize(self, _api_base: str) -> str:
        raise NotBuiltError(self.task)
