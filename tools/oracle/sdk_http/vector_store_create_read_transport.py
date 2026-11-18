from typing import ClassVar

from pydantic import Field

from not_built import Unbuilt


class CreateReadTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2485"

    store_create_read_transport: None = Field(alias="vector_store_create_read_transport")
