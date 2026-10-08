#!/usr/bin/env python3
"""Build internal release ZIPs with native provenance checks and cross-built candidates."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import struct
import subprocess
import tempfile
import zipfile

RELEASE_MATRIX = json.loads((pathlib.Path(__file__).resolve().parents[2]/'catalog/release-targets.json').read_text(encoding='utf-8'))
TARGETS = {item['target']: (item['os'], item['architecture']) for item in RELEASE_MATRIX['targets']}
FIELDS = ('cliVersion', 'buildCommit', 'buildTarget', 'localDevelopment', 'catalogVersion', 'catalogDigest', 'sourceVersion')


def binary_architecture(binary, system):
    with binary.open('rb') as file:
        header = file.read(64)
        if system == 'Windows':
            if len(header) < 64 or header[:2] != b'MZ':
                raise SystemExit('不是 Windows PE 二进制。')
            file.seek(struct.unpack_from('<I', header, 60)[0]); pe = file.read(6)
            if len(pe) != 6 or pe[:4] != b'PE\0\0':
                raise SystemExit('Windows PE 标头无效。')
            machine = struct.unpack_from('<H', pe, 4)[0]
            mapping = {0x8664: 'x86_64', 0xAA64: 'arm64'}
        elif system == 'Darwin':
            if len(header) < 8 or header[:4] != b'\xcf\xfa\xed\xfe':
                raise SystemExit('不是受支持的单架构 macOS Mach-O 二进制。')
            machine = struct.unpack_from('<I', header, 4)[0]
            mapping = {0x01000007: 'x86_64', 0x0100000c: 'arm64'}
        else:
            raise SystemExit('当前发行仅支持 macOS 与 Windows。')
        if machine not in mapping:
            raise SystemExit('不支持该程序架构。')
        return mapping[machine]


def validate(details, kind):
    if any(field not in details for field in FIELDS):
        raise SystemExit('缺少完整构建元数据；请重新 cargo build。')
    if details['buildTarget'] not in TARGETS or details['localDevelopment'] is not False:
        raise SystemExit('不分发其他目标或 local-dev 构建。')
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', str(details['cliVersion'])) or tuple(map(int, details['cliVersion'].split('.'))) < (0, 3, 0):
        raise SystemExit('新安装合同只接受 0.3.0 起的固定版本。')
    if not re.fullmatch(r'[0-9a-f]{40}' + (r'(?:\+dirty)?' if kind == 'candidate' else ''), str(details['buildCommit'])):
        raise SystemExit('正式发行必须来自干净 Git 提交；候选包可记录真实 dirty 提交。')
    if not re.fullmatch(r'[0-9a-f]{64}', str(details['catalogDigest'])) or not details['catalogVersion'] or not details['sourceVersion']:
        raise SystemExit('内置目录溯源信息无效。')


def native_probe(binary):
    if platform.system() not in ('Darwin', 'Windows'):
        raise SystemExit('内部稳定包的运行验收必须在目标平台进行。')
    body = json.loads(subprocess.run([str(binary), 'version'], capture_output=True, text=True, check=True, timeout=30).stdout)
    if body.get('ok') is not True or not isinstance(body.get('data'), dict):
        raise SystemExit('无法确认 dt-cli 版本。')
    return body['data']


def package(binary, output, kind='release', metadata=None):
    binary = binary.resolve()
    metadata = metadata or binary.parent / 'dt-cli.build.json'
    if kind == 'candidate':
        details = json.loads(metadata.read_text(encoding='utf-8'))
    else:
        details = native_probe(binary)
        if metadata.exists():
            recorded = json.loads(metadata.read_text(encoding='utf-8'))
            if any(details.get(key) != recorded.get(key) for key in FIELDS):
                raise SystemExit('实际程序与构建元数据不一致。')
    validate(details, kind)
    system, architecture = TARGETS[details['buildTarget']]
    if binary_architecture(binary, system) != architecture or (kind == 'release' and system != platform.system()):
        raise SystemExit('构建目标与程序架构或原生验收平台不一致。')
    name = 'dt-cli.exe' if system == 'Windows' else 'dt-cli'
    payload = binary.read_bytes()
    manifest = {
        'manifestSchemaVersion': 1, 'releaseType': kind, 'version': details['cliVersion'],
        'os': system, 'architecture': architecture, 'binary': name,
        'sha256': hashlib.sha256(payload).hexdigest(),
        **{key: details[key] for key in FIELDS if key != 'cliVersion'},
        'profileFormat': 1, 'credentialFormat': 1,
        'minimumInstallerSchema': 1, 'minimumLauncherSchema': 1,
        'nativeProbe': 'performed' if kind == 'release' else 'not-performed',
    }
    source = pathlib.Path(__file__).resolve().parent
    files = {name: payload, 'manifest.json': (json.dumps(manifest, ensure_ascii=False, indent=2)+'\n').encode('utf-8'),
             'install.sh': (source/'install.sh').read_bytes(), 'install.ps1': (source/'install.ps1').read_bytes()}
    output.parent.mkdir(parents=True, exist_ok=True)
    handle, temp = tempfile.mkstemp(dir=output.parent, prefix='.dt-cli-package-'); os.close(handle)
    try:
        with zipfile.ZipFile(temp, 'w', zipfile.ZIP_DEFLATED) as archive:
            for path, content in files.items():
                item = zipfile.ZipInfo(path, (2026, 1, 1, 0, 0, 0)); item.compress_type = zipfile.ZIP_DEFLATED
                item.external_attr = (0o100755 if path in (name, 'install.sh') else 0o100644) << 16
                archive.writestr(item, content)
        os.replace(temp, output)
    finally:
        pathlib.Path(temp).unlink(missing_ok=True)
    output.with_suffix('.manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(output.suffix+'.sha256').write_text(digest+'  '+output.name+'\n', encoding='utf-8')
    print(f'已生成 {kind} 包 {output}（{details["cliVersion"]}，{system}/{architecture}）；ZIP SHA256：{digest}')
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--kind', choices=('candidate', 'release'), default='release')
    parser.add_argument('--metadata', type=pathlib.Path)
    args = parser.parse_args(); package(args.binary, args.output, args.kind, args.metadata)


if __name__ == '__main__':
    main()
