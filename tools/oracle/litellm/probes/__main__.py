import os
import sys

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from probes.protocol import serve
from probes.registry import OPERATIONS

serve(sys.stdin.buffer, sys.stdout, OPERATIONS)
