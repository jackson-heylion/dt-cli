#!/usr/bin/env python3
"""Verify a release, atomically replace its executable and retain one rollback binary."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import tempfile
import zipfile


def verify_checksum(path, expected):
    if not expected or len(expected) != 64 or any(c not in '0123456789abcdef' for c in expected):
        raise SystemExit('请提供可信入口公布的 --sha256；安装/升级为整个 ZIP，回退为备份程序摘要。')
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise SystemExit('文件摘要与可信入口公布值不一致。')


def probe(binary):
    result = subprocess.run([str(binary), 'version'], capture_output=True, text=True, check=True)
    body = json.loads(result.stdout)
    if body.get('ok') is not True or not isinstance(body.get('data'), dict):
        raise SystemExit('无法确认程序版本。')
    return body


def host_architecture():
    return {'amd64': 'x86_64', 'x86_64': 'x86_64', 'arm64': 'arm64', 'aarch64': 'arm64'}.get(platform.machine().lower(), platform.machine())


def verify_manifest(manifest, body):
    if manifest.get('releaseType', 'release') != 'release':
        raise SystemExit('候选包须使用独立候选安装目录与内置安装器；不能覆盖正式程序。')
    details = body.get('data')
    if body.get('ok') is not True or not isinstance(details, dict) or details.get('cliVersion') != manifest.get('version'):
        raise SystemExit('程序版本与发行包清单不一致。')
    version = manifest['version']
    parts = version.split('.') if isinstance(version, str) else []
    if len(parts) != 3 or any(not part.isdigit() for part in parts):
        raise SystemExit('发行版本格式无效。')
    if 'manifestSchemaVersion' in manifest:
        if manifest['manifestSchemaVersion'] != 1 or any(manifest.get(field) != 1 for field in ('profileFormat', 'credentialFormat', 'minimumInstallerSchema', 'minimumLauncherSchema')) or manifest.get('nativeProbe') != 'performed':
            raise SystemExit('安装清单或配置/凭证格式不兼容；需要单独迁移。')
        if manifest.get('localDevelopment') is not False or any(manifest.get(field) != details.get(field) for field in ('buildTarget', 'localDevelopment')):
            raise SystemExit('程序构建目标或开发模式与清单不一致。')
    fields = ('buildCommit', 'catalogVersion', 'catalogDigest', 'sourceVersion')
    # 0.2.x packages predate these manifest fields; compare any they do carry.
    if tuple(map(int, parts)) >= (0, 3, 0) and any(not manifest.get(field) for field in fields):
        raise SystemExit('新版发行包缺少源码或目录溯源信息。')
    if tuple(map(int, parts)) >= (0, 3, 0):
        for field, length in [('buildCommit', 40), ('catalogDigest', 64)]:
            value = manifest[field]
            if not isinstance(value, str) or len(value) != length or any(c not in '0123456789abcdef' for c in value):
                raise SystemExit('新版发行包的源码提交或目录摘要无效。')
    for field in fields:
        if field in manifest and details.get(field) != manifest[field]:
            raise SystemExit('程序源码或目录信息与发行包清单不一致。')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--package', type=pathlib.Path)
    parser.add_argument('--destination', type=pathlib.Path, required=True)
    parser.add_argument('--rollback', action='store_true')
    parser.add_argument('--sha256', help='可信入口公布的 ZIP 摘要；回退时为备份程序摘要')
    parser.add_argument('--signer', help='兼容旧命令，内部发行不再核验发行者签名')
    args = parser.parse_args()
    destination = args.destination.expanduser().resolve()
    backup = destination.with_name(destination.name + '.previous')
    if args.rollback:
        if not backup.is_file():
            raise SystemExit('没有可回退版本。')
        verify_checksum(backup, args.sha256)
        probe(backup)
        os.replace(backup, destination)
        print('已回退，配置与凭证保持原位。')
        return
    if not args.package:
        raise SystemExit('请提供 --package。')
    verify_checksum(args.package, args.sha256)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent, prefix='.dt-cli-upgrade-') as folder:
        folder = pathlib.Path(folder)
        with zipfile.ZipFile(args.package) as archive:
            manifest = json.loads(archive.read('manifest.json'))
            name = manifest['binary']
            if pathlib.PurePath(name).name != name or '/' in name or '\\' in name:
                raise SystemExit('无效的发行包。')
            architecture = manifest['architecture']
            supported = architecture == host_architecture() or (
                platform.system() == 'Darwin' and architecture == 'universal2' and host_architecture() in {'arm64', 'x86_64'}
            )
            if manifest['os'] != platform.system() or not supported:
                raise SystemExit('发行包与当前系统或架构不匹配。')
            data = archive.read(name)
            if hashlib.sha256(data).hexdigest() != manifest['sha256']:
                raise SystemExit('发行包校验失败。')
            staged = folder / name
            staged.write_bytes(data)
            staged.chmod(0o755)
        # Probe before changing either the executable or its existing rollback artifact.
        body = probe(staged)
        verify_manifest(manifest, body)
        if destination.exists():
            staged_backup = folder / 'previous'
            shutil.copy2(destination, staged_backup)
            os.replace(staged_backup, backup)
        os.replace(staged, destination)
    print('升级完成；配置和系统安全存储保持原位。原程序保留为 .previous。')


if __name__ == '__main__':
    main()
