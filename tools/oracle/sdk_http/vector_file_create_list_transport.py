from typing import ClassVar

from pydantic import Field

from not_built import Unbuilt


class FileCreateListTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2480"

    file_create_list_transport: None = Field(alias="vector_file_create_list_transport")
