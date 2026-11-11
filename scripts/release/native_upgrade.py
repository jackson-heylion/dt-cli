#!/usr/bin/env python3
"""Check a real Windows stable → release candidate → stable rollback before promotion."""
import argparse
from dataclasses import dataclass
import json
import os
import pathlib
import platform
import re
import shutil
import sys
import tempfile
import urllib.request
import zipfile

from index import MAX_ZIP, digest, read_json, version
from native_ci import ROOT, data, run
from verify_public import NoRedirect

TARGET = 'x86_64-pc-windows-msvc'
JOB_ID = 'J' * 43
WRITE_ID = 'W' * 43


@dataclass(frozen=True)
class Stable:
    # 稳定通道序号，用于防止在验证后换了前版。
    sequence: int
    # 稳定发行版本。
    version: str
    # 对应不可变发行索引的 SHA-256。
    release_sha256: str

    @classmethod
    def from_json(cls, value):
        tag = value['version']
        version(tag)
        sequence = value['sequence']
        sha256 = value['releaseSha256']
        if (value['schemaVersion'] != 1 or type(sequence) is not int or sequence <= 0
                or value['releaseKey'] != f'releases/{tag}/native-release.json'
                or not valid_digest(sha256)):
            raise ValueError('Invalid native stable metadata')
        return cls(sequence, tag, sha256)

    def evidence(self):
        return dict(sequence=self.sequence, version=self.version, releaseSha256=self.release_sha256)


@dataclass(frozen=True)
class WindowsRelease:
    # 上一稳定发行通道。
    stable: Stable
    # 前版二进制构建来源提交。
    build_commit: str
    # 前版根命令目录摘要。
    catalog_digest: str
    # 前版原生 ZIP SHA-256。
    archive_sha256: str
    # 前版原生 ZIP 内容。
    archive: bytes


def valid_digest(value):
    return isinstance(value, str) and re.fullmatch(r'[0-9a-f]{64}', value) is not None


def download_from(base):
    opener = urllib.request.build_opener(NoRedirect())

    def fetch(key, limit):
        request = urllib.request.Request(base + key, headers={'Cache-Control': 'no-cache'})
        with opener.open(request, timeout=90) as response:
            raw = response.read(limit + 1)
        if len(raw) > limit:
            raise ValueError('Oversized public native artifact')
        return raw

    return fetch


def previous_windows(fetch, channel_key):
    stable = Stable.from_json(json.loads(fetch(channel_key, 16384)))
    prefix = f'releases/{stable.version}/'
    raw = fetch(prefix + 'native-release.json', 65536)
    if digest(raw) != stable.release_sha256:
        raise ValueError('Previous release index digest differs')
    release = json.loads(raw)
    if (release['schemaVersion'] != 1 or release['version'] != stable.version
            or not re.fullmatch(r'[0-9a-f]{40}', release['buildCommit'])
            or not valid_digest(release['catalogDigest'])):
        raise ValueError('Previous release provenance is invalid')
    packages = [item for item in release['packages'] if item['target'] == TARGET]
    key = prefix + 'dt-cli-windows-x64.zip'
    if len(packages) != 1:
        raise ValueError('Require one previous Windows release package')
    item = packages[0]
    if (item['key'] != key or item['os'] != 'Windows' or item['architecture'] != 'x86_64'
            or type(item['bytes']) is not int or not 0 < item['bytes'] <= MAX_ZIP
            or not valid_digest(item['sha256'])):
        raise ValueError('Invalid previous Windows package metadata')
    archive = fetch(key, item['bytes'])
    if len(archive) != item['bytes'] or digest(archive) != item['sha256']:
        raise ValueError('Previous Windows package digest or length differs')
    return WindowsRelease(stable, release['buildCommit'], release['catalogDigest'], item['sha256'], archive)


def require_verified_baseline(tree, current):
    candidate = Stable.from_json(read_json(tree / 'channels/native-stable.json'))
    evidence = read_json(tree / 'releases' / candidate.version / 'dt-cli-windows-x64.native-check.json')
    upgrade = evidence.get('windowsUpgrade')
    if (not upgrade or upgrade.get('candidateVersion') != candidate.version
            or upgrade.get('verified') is not True):
        raise ValueError('Missing Windows native upgrade and rollback evidence')
    prior = upgrade['previousStable']
    if version(prior['version']) >= version(candidate.version):
        raise ValueError('Windows upgrade did not verify an earlier stable release')
    # 发布任务重跑时，已切换到完全相同的候选仍可继续做公网验收。
    if current != candidate and current.evidence() != prior:
        raise ValueError('Stable changed after the Windows upgrade check; rebuild against current stable')


def check_launcher(launcher, tag, commit, catalog):
    probe = data([launcher, 'version'])
    expected = dict(cliVersion=tag, buildCommit=commit, catalogDigest=catalog,
                    buildTarget=TARGET, localDevelopment=False)
    if any(probe.get(key) != value for key, value in expected.items()):
        raise ValueError('Windows launcher did not run the expected published binary')


def check_records(launcher):
    profiles = data([launcher, 'profiles'])
    status = data([launcher, 'jobs', 'status', '--id', JOB_ID])
    page = data([launcher, 'jobs', 'results', '--id', JOB_ID])
    writes = data([launcher, 'writes', 'list'])
    if ([item['name'] for item in profiles['items']] != ['upgrade-fixture']
            or status['state'] != 'succeeded' or status['progress']['pages'] != 1
            or page['checkpointCommitted'] is not True
            or '123.0000000000000000000000001' not in page['page']['result']['body']
            or [(item['recordId'], item['stage']) for item in writes['items']]
               != [(WRITE_ID, 'result_unknown')]):
        raise ValueError('Windows upgrade fixture no longer returns the original records')


def verify(package, previous):
    if platform.system() != 'Windows':
        raise ValueError('The real upgrade check requires a native Windows runner')
    candidate = read_json(package.with_suffix('.manifest.json'))
    if version(candidate['version']) <= version(previous.stable.version):
        raise ValueError('The Windows release candidate must advance stable')
    roaming = run(['pwsh', '-NoProfile', '-NonInteractive', '-Command',
                   "[Environment]::GetFolderPath('ApplicationData')"]).stdout.strip()
    if not roaming or not pathlib.Path(roaming).is_absolute():
        raise ValueError('Windows runner has no per-user application data folder')
    config = pathlib.Path(roaming) / 'datousoft' / 'dt-cli' / 'config'
    # 发行二进制始终使用系统已知目录。只在隔离 runner 新建空目录，存在任何对象就停止。
    config.mkdir(parents=True, exist_ok=False)
    try:
        env = os.environ | {'DT_CLI_WINDOWS_UPGRADE_CONFIG': str(config)}
        run(['cargo', 'test', '--locked', '--features', 'local-dev', '--test', 'native_platform_records',
             'seed_windows_upgrade_records', '--', '--ignored', '--exact'], env=env)
        with tempfile.TemporaryDirectory(prefix='dtcli-windows-upgrade-') as folder:
            root = pathlib.Path(folder)
            old_archive = root / 'dt-cli-windows-x64.zip'
            old_archive.write_bytes(previous.archive)
            installer = root / 'install.ps1'
            with zipfile.ZipFile(old_archive) as release:
                entries = release.infolist()
                if len(entries) != 4 or {entry.filename for entry in entries} != {
                        'dt-cli.exe', 'manifest.json', 'install.sh', 'install.ps1'}:
                    raise ValueError('Unexpected previous Windows package entries')
                for name in ('install.ps1', 'dt-cli.exe'):
                    entry = release.getinfo(name)
                    if not 0 < entry.file_size <= MAX_ZIP:
                        raise ValueError('Invalid previous Windows installer or executable size')
                    (root / name).write_bytes(release.read(entry))
            installed = data(['pwsh', '-NoLogo', '-NoProfile', '-File', installer,
                              '--package', old_archive, '--sha256', previous.archive_sha256,
                              '--directory', root / 'managed installation'])
            launcher = pathlib.Path(installed['launcher'])
            check_launcher(launcher, previous.stable.version, previous.build_commit, previous.catalog_digest)
            check_records(launcher)
            switched = data([launcher, 'upgrade', '--package', package, '--sha256', digest(package.read_bytes())])
            if switched['changed'] is not True or switched['version'] != candidate['version']:
                raise ValueError('Windows upgrade did not select the candidate')
            check_launcher(launcher, candidate['version'], candidate['buildCommit'], candidate['catalogDigest'])
            check_records(launcher)
            status = data([launcher, 'upgrade', '--check'])
            if status['previous']['version'] != previous.stable.version:
                raise ValueError('Windows upgrade lost its previous version')
            rollback = data([launcher, 'upgrade', '--rollback'])
            if rollback['changed'] is not True or rollback['version'] != previous.stable.version:
                raise ValueError('Windows rollback did not select the previous version')
            check_launcher(launcher, previous.stable.version, previous.build_commit, previous.catalog_digest)
            check_records(launcher)
    finally:
        shutil.rmtree(config)
    return dict(verified=True, candidateVersion=candidate['version'], previousStable=previous.stable.evidence())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--package', type=pathlib.Path)
    mode.add_argument('--before-publish', type=pathlib.Path)
    args = parser.parse_args()
    distribution = read_json(ROOT / 'skills/dt-cli/scripts/distribution.json')
    fetch = download_from(distribution['publicBaseUrl'])
    channel = distribution['channelKey']
    if args.before_publish:
        require_verified_baseline(args.before_publish, Stable.from_json(json.loads(fetch(channel, 16384))))
    else:
        print(json.dumps(verify(args.package, previous_windows(fetch, channel))))


if __name__ == '__main__':
    main()
