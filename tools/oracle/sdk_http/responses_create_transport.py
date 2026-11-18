from typing import ClassVar

from not_built import Unbuilt


class CreateTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2414"

    responses_create_transport: None
