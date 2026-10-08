#!/usr/bin/env python3
"""Build, package and install a native release in a temporary CI directory; no IAM login."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import sys
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]


def run(argv, *, success=True):
    result = subprocess.run([str(a) for a in argv], cwd=ROOT, capture_output=True, text=True,
                            encoding='utf-8', timeout=1800)
    if success and result.returncode != 0:
        sys.stdout.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)
    return result


def data(argv):
    body = json.loads(run(argv).stdout)
    if body.get('ok') is not True or not isinstance(body.get('data'), dict):
        raise SystemExit('Native command did not return a successful JSON envelope.')
    return body['data']


def verify(target, name):
    matrix = json.loads((ROOT / 'catalog/release-targets.json').read_text(encoding='utf-8'))
    expected = next((item for item in matrix['targets'] if item['target'] == target), None)
    architecture = {'aarch64': 'arm64', 'amd64': 'x86_64'}.get(platform.machine().lower(), platform.machine().lower())
    if expected is None or (platform.system(), architecture) != (expected['os'], expected['architecture']):
        raise SystemExit('Native CI must run on the exact release OS and architecture.')
    if run(['git', 'status', '--porcelain']).stdout.strip():
        raise SystemExit('Native release requires a clean checkout.')
    commit = run(['git', 'rev-parse', 'HEAD']).stdout.strip()
    run(['cargo', 'build', '--release', '--locked', '--target', target])
    binary = ROOT / 'target' / target / 'release' / expected['binary']
    probe = data([binary, 'version'])
    if probe.get('buildCommit') != commit or probe.get('buildTarget') != target or probe.get('localDevelopment') is not False:
        raise SystemExit('Native executable provenance does not match this checkout.')
    folder = ROOT / 'output/native'
    folder.mkdir(parents=True, exist_ok=True)
    archive = folder / f'dt-cli-{name}.zip'
    run([sys.executable, ROOT / 'scripts/release/package.py', '--binary', binary,
         '--output', archive, '--kind', 'release'])
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    with tempfile.TemporaryDirectory(prefix='dt-cli-native-') as temporary:
        temporary = pathlib.Path(temporary)
        extracted = temporary / 'package'
        installation = temporary / 'installation with spaces'
        with zipfile.ZipFile(archive) as package:
            if set(package.namelist()) != {expected['binary'], 'manifest.json', 'install.sh', 'install.ps1'}:
                raise SystemExit('Native package has unexpected entries.')
            package.extractall(extracted)
        if platform.system() == 'Darwin':
            (extracted / expected['binary']).chmod(0o755)
            installer = ['sh', extracted / 'install.sh']
        else:
            installer = ['pwsh', '-NoLogo', '-NoProfile', '-File', extracted / 'install.ps1']
        installed = data([*installer, '--package', archive, '--sha256', digest, '--directory', installation])
        launcher = pathlib.Path(installed['launcher'])
        installed_probe = data([launcher, 'version'])
        if installed_probe != probe:
            raise SystemExit('Installed launcher does not return the original executable metadata.')
        rejected = run([launcher, 'upgrade', '--package', archive, '--sha256', '0' * 64], success=False)
        if rejected.returncode == 0 or data([launcher, 'version']) != probe:
            raise SystemExit('Bad ZIP digest did not preserve the installed executable.')
        checked = data([launcher, 'upgrade', '--package', archive, '--sha256', digest, '--check'])
        if checked.get('packageVerified') is not True or checked.get('changed') is not False:
            raise SystemExit('Local package check failed or mutated installation.')
    evidence = {'schemaVersion': 1, 'version': probe['cliVersion'], 'buildCommit': commit,
                'target': target, 'os': platform.system(), 'architecture': architecture,
                'archiveSha256': digest, 'nativeProbe': 'performed',
                'checks': ['package', 'installer', 'launcher', 'bad-zip-preserves-installation', 'local-upgrade-check'],
                'iamLoginPerformed': False}
    (folder / f'dt-cli-{name}.native-check.json').write_text(json.dumps(evidence, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(evidence))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--name', choices=('macos-arm64', 'windows-x64'), required=True)
    args = parser.parse_args()
    os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
    verify(args.target, args.name)


if __name__ == '__main__':
    main()
