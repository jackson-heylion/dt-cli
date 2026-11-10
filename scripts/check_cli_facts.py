#!/usr/bin/env python3
"""核验命令树、运行元数据与 Skill 版本事实；只读，不登录或发行。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def invoke(binary, *args):
    result = subprocess.run([str(binary), *args], capture_output=True, text=True, check=True)
    value = json.loads(result.stdout)
    if not value.get('ok'):
        raise ValueError('CLI offline fact query failed')
    return value['data']


def check(binary):
    version = invoke(binary, 'version')
    help_data = invoke(binary, 'help')
    expected = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    if version['cliVersion'] != expected:
        raise ValueError('Running version differs from source version')
    facts = {key: help_data[key] for key in ('version', 'sourceVersion', 'commands')}
    encoded = json.dumps(facts, ensure_ascii=False, separators=(',', ':'), sort_keys=True).encode()
    if hashlib.sha256(encoded).hexdigest() != version['catalogDigest']:
        raise ValueError('Help definition differs from build command facts')
    commands = {row['command'] for row in facts['commands']}
    if {'dt-cli call', 'dt-cli writes dispatch', 'dt-cli manage apply'} - commands:
        raise ValueError('Current root commands missing')
    for directory in (ROOT / 'skills').iterdir():
        if not directory.is_dir():
            continue
        config = json.loads((directory / 'scripts/distribution.json').read_text())
        entry = (directory / 'SKILL.md').read_text()
        match = re.search(r'^\s+version: "([^"]+)"', entry, re.M)
        if not match or match[1] != config['skillVersion']:
            raise ValueError('Skill entry and distribution versions differ: ' + directory.name)
        if config['minimumCliVersion'] != expected:
            raise ValueError('Skill minimum does not require current root commands: ' + directory.name)
        for document in directory.rglob('*.md'):
            text = document.read_text()
            if re.search(r'(?:dt-cli|<launcher>)\s+platform\s+(?:login|call|jobs|writes|manage)', text):
                raise ValueError('Retired CLI prefix in Skill: ' + str(document.relative_to(ROOT)))
            for target in re.findall(r'\[[^\]]*\]\(([^)]+)\)', text):
                path = target.split('#', 1)[0]
                if path and not re.match(r'[a-z]+://', path) and not (document.parent / path).exists():
                    raise ValueError('Missing Skill reference: ' + str(document.relative_to(ROOT)) + ': ' + path)
    print(json.dumps({'sourceVersion': expected, 'runningVersion': version['cliVersion'],
                      'commandDefinitions': len(commands), 'skillVersionsVerified': True,
                      'publicStableVerified': False}, ensure_ascii=False))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/dt-cli')
    check(parser.parse_args().binary.resolve())
