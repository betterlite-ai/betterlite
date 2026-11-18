from typing import ClassVar

from not_built import Unbuilt


class ResponsesPublicDispatchRequest(Unbuilt):
    task: ClassVar[str] = "B080"

    responses_public_dispatch: None
