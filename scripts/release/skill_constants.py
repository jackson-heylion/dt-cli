"""Shared immutable package and channel rules for all skills."""
import re

DEFAULT_SKILL = 'dt-cli'
SKILL_NAME = re.compile(r'[a-z0-9]+(?:-[a-z0-9]+)*')
SEMANTIC_VERSION = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)')
MAX_SKILL_NAME_LENGTH = 63
MAX_PACKAGE_BYTES = 16 * 1024 * 1024
MAX_FILE_BYTES = 1024 * 1024
MAX_FILES = 128
MAX_JSON_BYTES = 16 * 1024
PUBLIC_TIMEOUT_SECONDS = 90
PUBLIC_READBACK_ATTEMPTS = 30
PUBLIC_READBACK_SECONDS = 2
PACKAGE_TIMESTAMP = (2026, 1, 1, 0, 0, 0)
PORTABLE_FILE_MODE = 0o100644
RESOURCE_DIRECTORIES = ('references', 'scripts', 'assets', 'templates')
ROOT_FILES = ('SKILL.md', 'LICENSE', '.skill-metadata.yaml')
EXCLUDED_PARTS = frozenset(('__pycache__', '.git', '.venv', 'node_modules', 'runs', 'runtime', 'output', 'tmp'))
EXCLUDED_SUFFIXES = frozenset(('.pyc', '.pyo', '.log', '.pem', '.key'))
DEFAULT_STABLE_ARCHIVE = 'skills/stable/dt-cli-skill.zip'


def skill_keys(name, version):
    if not isinstance(name, str) or not SKILL_NAME.fullmatch(name) or len(name) > MAX_SKILL_NAME_LENGTH:
        raise ValueError('Invalid Skill name')
    prefix = 'skills' if name == DEFAULT_SKILL else f'skills/{name}'
    channel = 'channels/skill-stable.json' if name == DEFAULT_SKILL else f'channels/skills/{name}-stable.json'
    stable = DEFAULT_STABLE_ARCHIVE if name == DEFAULT_SKILL else f'skills/{name}/stable/{name}-skill.zip'
    return channel, f'{prefix}/{version}/release.json', f'{prefix}/{version}/{name}-skill.zip', stable
