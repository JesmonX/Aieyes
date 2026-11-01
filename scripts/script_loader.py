"""Shared helper for tests that import hyphenated or standalone scripts by path."""
import importlib.util
import pathlib

SCRIPTS = pathlib.Path(__file__).resolve().parent


def load_script(name, path):
    """Execute a script file as a fresh module; relative paths resolve from scripts/."""
    path = SCRIPTS / path
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
