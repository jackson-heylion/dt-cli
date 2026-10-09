#!/usr/bin/env python3
"""Publish the universal Skill independently of already released native CLI packages."""
import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import zipfile

from package_skill import package
from publish_qiniu import QiniuStore, configuration, sha, publish_immutable

ROOT = pathlib.Path(__file__).resolve().parents[2]
STABLE_ARCHIVE_KEY = 'skills/stable/dt-cli-skill.zip'


def assemble(output):
    import yaml
    source = ROOT / 'skills/dt-cli'
    config = json.loads((source / 'scripts/distribution.json').read_bytes())
    version = config['skillVersion']
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', version):
        raise ValueError('Invalid Skill version')
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT).strip():
        raise ValueError('Skill release requires a clean source checkout')
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if output.exists() and any(output.iterdir()):
        raise ValueError('Skill output must be empty')
    archive = output / f'skills/{version}/dt-cli-skill.zip'
    package(source, archive)
    with zipfile.ZipFile(archive) as bundle:
        entry = bundle.read('dt-cli/SKILL.md').decode('utf-8')
        header = yaml.safe_load(entry.split('---', 2)[1])
        if header['metadata']['version'] != version or header['name'] != 'dt-cli':
            raise ValueError('Skill source and packaged version disagree')
        if header['agent_created'] is not True or header['user-invocable'] is not True:
            raise ValueError('Missing host invocation metadata')
        for field in ('name_en', 'name_zh', 'description', 'description_en', 'description_zh',
                      'argument-hint', 'argument-hint-en', 'argument-hint-zh'):
            if not isinstance(header.get(field), str) or not header[field].strip():
                raise ValueError('Missing localized Skill metadata')
        examples = yaml.safe_load(bundle.read('dt-cli/.skill-metadata.yaml'))['examples']
        for example in examples:
            for field in ('title', 'description', 'prompt'):
                if not all(example[field].get(language) for language in ('zh', 'en')):
                    raise ValueError('Missing localized recommended task')
        yaml.safe_load(bundle.read('dt-cli/agents/openai.yaml'))
    data = archive.read_bytes()
    index = dict(schemaVersion=1, skill='dt-cli', version=version, sourceCommit=commit,
                 key=archive.relative_to(output).as_posix(), sha256=sha(data), bytes=len(data),
                 minimumCliVersion=config['minimumCliVersion'], bootstrapSchema=config['bootstrapSchema'],
                 clients=['Codex', 'Claude Code', 'WorkBuddy', 'QwenWork'])
    release_key = f'skills/{version}/release.json'
    release_data = (json.dumps(index, indent=2) + '\n').encode()
    (output / release_key).write_bytes(release_data)
    channel = dict(schemaVersion=1, skill='dt-cli', version=version,
                   releaseKey=release_key, releaseSha256=sha(release_data))
    (output / 'channels').mkdir()
    (output / 'channels/skill-stable.json').write_text(json.dumps(channel, indent=2) + '\n')
    return channel


def publish(output, store, wait_for_readback=False):
    channel_key = 'channels/skill-stable.json'
    channel_bytes = (output / channel_key).read_bytes()
    channel = json.loads(channel_bytes)
    release_key = f'skills/{channel["version"]}/release.json'
    release_data = (output / release_key).read_bytes()
    if channel['releaseKey'] != release_key or sha(release_data) != channel['releaseSha256']:
        raise ValueError('Invalid Skill channel')
    release = json.loads(release_data)
    archive = output / f'skills/{channel["version"]}/dt-cli-skill.zip'
    data = archive.read_bytes()
    if release['version'] != channel['version'] or release['key'] != archive.relative_to(output).as_posix() or release['sha256'] != sha(data) or release['bytes'] != len(data):
        raise ValueError('Skill package differs from immutable index')
    before = store.read(channel_key, 16384)
    if before:
        previous = json.loads(before)
        if (previous.get('schemaVersion') != 1 or previous.get('skill') != 'dt-cli'
                or not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', previous.get('version', ''))
                or previous.get('releaseKey') != f"skills/{previous['version']}/release.json"
                or not re.fullmatch(r'[0-9a-f]{64}', previous.get('releaseSha256', ''))):
            raise ValueError('Invalid current Skill channel')
        if tuple(map(int, previous['version'].split('.'))) > tuple(map(int, channel['version'].split('.'))) or previous['version'] == channel['version'] and before != channel_bytes:
            raise ValueError('Skill downgrade or same-version replacement blocked')

    def verify(key, expected, maximum=None):
        print(json.dumps({'stage': 'public-readback', 'key': key}), flush=True)
        maximum = maximum or len(expected)
        actual = store.read(key, maximum)
        if wait_for_readback:
            for _ in range(30):
                if actual == expected:
                    break
                time.sleep(2)
                actual = store.read(key, maximum)
        if actual != expected:
            raise ValueError('Public Skill readback differs: ' + key)

    publish_immutable(output, [archive, archive.with_suffix('.manifest.json'), output / release_key],
                      store, wait_for_readback)
    if store.read(channel_key, 16384) != before:
        raise ValueError('Skill channel changed during publication')
    # This import URL follows the verified release; the versioned ZIP stays immutable.
    # Allow a previous ZIP to be larger than the new one when comparing mutable bytes.
    stable_maximum = max(len(data), 16 * 1024 * 1024)
    if store.read(STABLE_ARCHIVE_KEY, stable_maximum) != data:
        store.upload(STABLE_ARCHIVE_KEY, archive, immutable=False)
    verify(STABLE_ARCHIVE_KEY, data, stable_maximum)
    if store.read(channel_key, 16384) != before:
        raise ValueError('Skill channel changed during publication')
    if before != channel_bytes:
        store.upload(channel_key, output / channel_key, immutable=False)
    verify(channel_key, channel_bytes)
    return dict(published=True, skillVersion=channel['version'], clients=release['clients'],
                key=release['key'], stableKey=STABLE_ARCHIVE_KEY, sha256=release['sha256'], nativeCliChanged=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    parser.add_argument('--repair-stable-cache', action='store_true',
                        help='Repair cache headers of the already published stable ZIP and Skill channel')
    args = parser.parse_args()
    try:
        if not args.repair_stable_cache:
            if args.output is None:
                parser.error('--output is required for publication')
            assemble(args.output)
        base, bucket, region = (os.environ[name] for name in ('QINIU_PUBLIC_BASE_URL', 'QINIU_BUCKET', 'QINIU_REGION'))
        prefix = configuration(base, bucket, region)
        store = QiniuStore(bucket, region, base, prefix)
        if args.repair_stable_cache:
            channel_bytes = store.read('channels/skill-stable.json', 16384)
            channel = json.loads(channel_bytes)
            if channel['releaseKey'] != f'skills/{channel["version"]}/release.json':
                raise ValueError('Invalid current Skill channel')
            release_bytes = store.read(channel['releaseKey'], 16384)
            if sha(release_bytes) != channel['releaseSha256']:
                raise ValueError('Current Skill release digest differs')
            release = json.loads(release_bytes)
            if (release['key'] != f'skills/{channel["version"]}/dt-cli-skill.zip'
                    or sha(store.read(STABLE_ARCHIVE_KEY, release['bytes'])) != release['sha256']):
                raise ValueError('Current stable ZIP differs from the immutable release')
            for key in (STABLE_ARCHIVE_KEY, 'channels/skill-stable.json'):
                store.revalidate(key)
            if store.read('channels/skill-stable.json', 16384) != channel_bytes:
                raise ValueError('Skill channel changed during cache repair')
            print(json.dumps(dict(cacheRevalidationVerified=True, version=channel['version'],
                                  stableKey=STABLE_ARCHIVE_KEY, packageBytesChanged=False)))
        else:
            print(json.dumps(publish(args.output, store, True)))
    except Exception as error:
        if type(error) is ValueError:
            print(str(error), file=sys.stderr)
        print('SKILL_PUBLISH_FAILED: inspect package metadata and immutable public readback.', file=sys.stderr)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
