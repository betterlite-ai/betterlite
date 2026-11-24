import sys

from betterlite.assistants import _utils as utils
from betterlite.assistants.main import *  # noqa: F403

sys.modules[f"{__name__}.utils"] = utils
