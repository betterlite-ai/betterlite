from pathlib import Path

PROJECT = Path(__file__).resolve().parent
PYTHON = ("uv", "run", "--locked", "--project", str(PROJECT), "python")
