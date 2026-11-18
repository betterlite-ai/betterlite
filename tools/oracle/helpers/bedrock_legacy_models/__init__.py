from typing import TypedDict

import litellm
from pydantic import JsonValue as Json

from .constructors import constructors
from .mapping import mappings
from .output import output
from .projection import projection

litellm.suppress_debug_info = True


class Steps(TypedDict, total=False):
    mappings: Json
    constructors: Json
    projection: Json
    output_text: Json


def run(argument: Steps) -> Json:
    if "mappings" in argument:
        return mappings(argument["mappings"])
    if "constructors" in argument:
        return constructors(argument["constructors"])
    if "projection" in argument:
        return projection(argument["projection"])
    if "output_text" in argument:
        return output(argument["output_text"])
    raise ValueError(f"unsupported step {sorted(argument)}")
