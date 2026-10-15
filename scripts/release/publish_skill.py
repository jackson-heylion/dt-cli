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
from publish_qiniu import QiniuStore, configuration, sha, publish_immutable, timed

ROOT = pathlib.Path(__file__).resolve().parents[2]
from skill_constants import (DEFAULT_SKILL, DEFAULT_STABLE_ARCHIVE, skill_keys, MAX_JSON_BYTES,
                             MAX_PACKAGE_BYTES, PUBLIC_READBACK_ATTEMPTS, PUBLIC_READBACK_SECONDS)
STABLE_ARCHIVE_KEY = DEFAULT_STABLE_ARCHIVE


def assemble(output, skill_name=DEFAULT_SKILL):
    import yaml
    source = ROOT / 'skills' / skill_name
    channel_key, release_key, archive_key, _ = skill_keys(skill_name, 'unused')
    config = json.loads((source / 'scripts/distribution.json').read_bytes())
    version = config['skillVersion']
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', version):
        raise ValueError('Invalid Skill version')
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT).strip():
        raise ValueError('Skill release requires a clean source checkout')
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if output.exists() and any(output.iterdir()):
        raise ValueError('Skill output must be empty')
    channel_key, release_key, archive_key, _ = skill_keys(skill_name, version)
    archive = output / archive_key
    package(source, archive)
    with zipfile.ZipFile(archive) as bundle:
        entry = bundle.read(f'{skill_name}/SKILL.md').decode('utf-8')
        header = yaml.safe_load(entry.split('---', 2)[1])
        if header['metadata']['version'] != version or header['name'] != skill_name:
            raise ValueError('Skill source and packaged version disagree')
        if skill_name == DEFAULT_SKILL:
            if header['agent_created'] is not True or header['user-invocable'] is not True:
                raise ValueError('Missing host invocation metadata')
            for field in ('name_en', 'name_zh', 'description', 'description_en', 'description_zh',
                          'argument-hint', 'argument-hint-en', 'argument-hint-zh'):
                if not isinstance(header.get(field), str) or not header[field].strip():
                    raise ValueError('Missing localized Skill metadata')
            examples = yaml.safe_load(bundle.read(f'{skill_name}/.skill-metadata.yaml'))['examples']
            for example in examples:
                for field in ('title', 'description', 'prompt'):
                    if not all(example[field].get(language) for language in ('zh', 'en')):
                        raise ValueError('Missing localized recommended task')
            yaml.safe_load(bundle.read(f'{skill_name}/agents/openai.yaml'))
        yaml.safe_load(bundle.read(f'{skill_name}/agents/openai.yaml'))
    data = archive.read_bytes()
    index = dict(schemaVersion=1, skill=skill_name, version=version, sourceCommit=commit,
                 key=archive.relative_to(output).as_posix(), sha256=sha(data), bytes=len(data),
                 minimumCliVersion=config['minimumCliVersion'], bootstrapSchema=config['bootstrapSchema'],
                 clients=['Codex', 'Claude Code', 'WorkBuddy', 'QwenWork'])
    release_key = skill_keys(skill_name, version)[1]
    release_data = (json.dumps(index, indent=2) + '\n').encode()
    (output / release_key).write_bytes(release_data)
    channel = dict(schemaVersion=1, skill=skill_name, version=version,
                   releaseKey=release_key, releaseSha256=sha(release_data))
    (output / channel_key).parent.mkdir(parents=True, exist_ok=True)
    (output / channel_key).write_text(json.dumps(channel, indent=2) + '\n')
    return channel


def publish(output, store, wait_for_readback=False, skill_name=DEFAULT_SKILL):
    channel_key = skill_keys(skill_name, 'unused')[0]
    channel_bytes = (output / channel_key).read_bytes()
    channel = json.loads(channel_bytes)
    _, release_key, archive_key, stable_archive_key = skill_keys(skill_name, channel["version"])
    release_data = (output / release_key).read_bytes()
    if channel.get('skill') != skill_name or channel['releaseKey'] != release_key or sha(release_data) != channel['releaseSha256']:
        raise ValueError('Invalid Skill channel')
    release = json.loads(release_data)
    archive = output / archive_key
    data = archive.read_bytes()
    if release['version'] != channel['version'] or release['key'] != archive.relative_to(output).as_posix() or release['sha256'] != sha(data) or release['bytes'] != len(data):
        raise ValueError('Skill package differs from immutable index')
    before = store.read(channel_key, MAX_JSON_BYTES)
    if before:
        previous = json.loads(before)
        if (previous.get('schemaVersion') != 1 or previous.get('skill') != skill_name
                or not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', previous.get('version', ''))
                or previous.get('releaseKey') != skill_keys(skill_name, previous['version'])[1]
                or not re.fullmatch(r'[0-9a-f]{64}', previous.get('releaseSha256', ''))):
            raise ValueError('Invalid current Skill channel')
        if tuple(map(int, previous['version'].split('.'))) > tuple(map(int, channel['version'].split('.'))) or previous['version'] == channel['version'] and before != channel_bytes:
            raise ValueError('Skill downgrade or same-version replacement blocked')

    def verify(key, expected, maximum=None):
        print(json.dumps({'stage': 'public-readback', 'key': key}), flush=True)
        maximum = maximum or len(expected)
        actual = store.read(key, maximum)
        if wait_for_readback:
            for _ in range(PUBLIC_READBACK_ATTEMPTS):
                if actual == expected:
                    break
                time.sleep(PUBLIC_READBACK_SECONDS)
                actual = store.read(key, maximum)
        if actual != expected:
            raise ValueError('Public Skill readback differs: ' + key)

    publish_immutable(output, [archive, archive.with_suffix('.manifest.json'), output / release_key],
                      store, wait_for_readback)
    with timed("skill-channel-switch"):
        if store.read(channel_key, MAX_JSON_BYTES) != before:
            raise ValueError('Skill channel changed during publication')
        # This import URL follows the verified release; the versioned ZIP stays immutable.
        # Allow a previous ZIP to be larger than the new one when comparing mutable bytes.
        stable_maximum = max(len(data), MAX_PACKAGE_BYTES)
        if store.read(stable_archive_key, stable_maximum) != data:
            store.upload(stable_archive_key, archive, immutable=False)
        verify(stable_archive_key, data, stable_maximum)
        if store.read(channel_key, MAX_JSON_BYTES) != before:
            raise ValueError('Skill channel changed during publication')
        if before != channel_bytes:
            store.upload(channel_key, output / channel_key, immutable=False)
        verify(channel_key, channel_bytes)
    return dict(published=True, skillVersion=channel['version'], clients=release['clients'],
                key=release['key'], stableKey=stable_archive_key, sha256=release['sha256'], nativeCliChanged=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    parser.add_argument('--skill', default=DEFAULT_SKILL)
    parser.add_argument('--repair-stable-cache', action='store_true',
                        help='Repair cache headers of the already published stable ZIP and Skill channel')
    args = parser.parse_args()
    try:
        if not args.repair_stable_cache:
            if args.output is None:
                parser.error('--output is required for publication')
            assemble(args.output, args.skill)
        base, bucket, region = (os.environ[name] for name in ('QINIU_PUBLIC_BASE_URL', 'QINIU_BUCKET', 'QINIU_REGION'))
        prefix = configuration(base, bucket, region)
        store = QiniuStore(bucket, region, base, prefix)
        if args.repair_stable_cache:
            channel_key = skill_keys(args.skill, 'unused')[0]
            channel_bytes = store.read(channel_key, MAX_JSON_BYTES)
            channel = json.loads(channel_bytes)
            _, release_key, archive_key, stable_key = skill_keys(args.skill, channel['version'])
            if channel['releaseKey'] != release_key:
                raise ValueError('Invalid current Skill channel')
            release_bytes = store.read(channel['releaseKey'], MAX_JSON_BYTES)
            if sha(release_bytes) != channel['releaseSha256']:
                raise ValueError('Current Skill release digest differs')
            release = json.loads(release_bytes)
            if (release['key'] != archive_key
                    or sha(store.read(stable_key, release['bytes'])) != release['sha256']):
                raise ValueError('Current stable ZIP differs from the immutable release')
            for key in (stable_key, channel_key):
                store.revalidate(key)
            if store.read(channel_key, MAX_JSON_BYTES) != channel_bytes:
                raise ValueError('Skill channel changed during cache repair')
            print(json.dumps(dict(cacheRevalidationVerified=True, version=channel['version'],
                                  stableKey=stable_key, packageBytesChanged=False)))
        else:
            with timed("skill-publication"):
                print(json.dumps(publish(args.output, store, True, args.skill)))
    except Exception as error:
        if type(error) is ValueError:
            print(str(error), file=sys.stderr)
        print('SKILL_PUBLISH_FAILED: inspect package metadata and immutable public readback.', file=sys.stderr)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
