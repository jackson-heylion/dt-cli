#!/usr/bin/env python3
"""Validate actual public release/Skill bytes and first installation on a native runner."""
import hashlib
import argparse
import json
import pathlib
import platform
import subprocess
import tempfile
import time
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def verify(skill_only=False, legacy_skill=False):
    config = json.loads((ROOT / 'skills/dt-cli/scripts/distribution.json').read_bytes())
    base = config['publicBaseUrl']
    opener = urllib.request.build_opener(NoRedirect())

    def download(key, limit):
        if not key.startswith(('channels/', 'releases/', 'skills/')) or any(p in ('', '.', '..') for p in key.split('/')):
            raise ValueError('Invalid public object key')
        request = urllib.request.Request(base + key, headers={'Cache-Control': 'no-cache'})
        for attempt in range(16):
            try:
                with opener.open(request, timeout=90) as response:
                    data = response.read(limit + 1)
                    if len(data) > limit:
                        raise ValueError('Oversized public object')
                    return data
            except urllib.error.HTTPError as error:
                if error.code != 404 or attempt == 15:
                    raise
                time.sleep(2)

    stable = json.loads(download(config['channelKey'], 16384))
    raw = download(stable['releaseKey'], 65536)
    if hashlib.sha256(raw).hexdigest() != stable['releaseSha256']:
        raise ValueError('Published release index digest differs')
    release = json.loads(raw)
    commit = release['buildCommit'] if skill_only else subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if release['buildCommit'] != commit or release['version'] != stable['version']:
        raise ValueError('Published release is not this verified source commit')
    if legacy_skill:
        skill = dict(key='skills/0.4.4/dt-cli-skill.zip', bytes=48341,
                     sha256='0cf9c84f6b270ca80e0501fa85d688c877754ed9fd603ca9bdca2231984474cc')
    elif skill_only:
        channel = json.loads(download('channels/skill-stable.json', 16384))
        raw_skill = download(channel['releaseKey'], 16384)
        if hashlib.sha256(raw_skill).hexdigest() != channel['releaseSha256']:
            raise ValueError('Universal Skill index digest differs')
        skill = json.loads(raw_skill)
        if skill['version'] != config['skillVersion'] or skill['version'] != channel['version']:
            raise ValueError('Universal Skill version differs')
        source_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        if skill['sourceCommit'] != source_commit:
            raise ValueError('Universal Skill was not published from this source commit')
    else:
        skill = next(item for item in release['skills'] if item['key'].endswith('/dt-cli-skill.zip'))
    skill_bytes = download(skill['key'], skill['bytes'])
    if len(skill_bytes) != skill['bytes'] or hashlib.sha256(skill_bytes).hexdigest() != skill['sha256']:
        raise ValueError('Published Skill digest differs')
    if skill_only:
        stable_skill_bytes = download('skills/stable/dt-cli-skill.zip', skill['bytes'])
        if stable_skill_bytes != skill_bytes:
            raise ValueError('Stable Skill download differs from the immutable release')
        skill_bytes = stable_skill_bytes
    with tempfile.TemporaryDirectory(prefix='dt-cli-public-') as temporary:
        temporary = pathlib.Path(temporary).resolve()
        archive = temporary / 'skill.zip'
        archive.write_bytes(skill_bytes)
        extracted = temporary / 'skill'
        with zipfile.ZipFile(archive) as package:
            for item in package.infolist():
                path = pathlib.PurePosixPath(item.filename)
                if not item.filename.startswith('dt-cli/') or path.is_absolute() or '..' in path.parts or item.file_size > 1024 * 1024 or (item.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError('Invalid Skill entries')
            package.extractall(extracted)
        scripts = extracted / 'dt-cli/scripts'
        installed_config = json.loads((scripts / 'distribution.json').read_bytes())
        if not legacy_skill and installed_config != config:
            raise ValueError('Published Skill has a different fixed prefix')
        if legacy_skill and (installed_config['publicBaseUrl'] != base or installed_config['channelKey'] != 'channels/stable.json'):
            raise ValueError('Legacy Skill does not use its original fixed channel')
        installation = temporary / 'installation 中文 with spaces'
        argv = ['bash', scripts / 'bootstrap.sh', '--directory', installation] if platform.system() == 'Darwin' else [
            'powershell', '-NonInteractive', '-NoProfile', '-File', scripts / 'bootstrap.ps1', '-Directory', installation]

        def run(args):
            result = subprocess.run([str(a) for a in args], capture_output=True, text=True, encoding='utf-8', timeout=240)
            if result.returncode != 0:
                raise ValueError(f'Public bootstrap failed: {result.stdout} {result.stderr}')
            envelope = json.loads(result.stdout)
            if envelope.get('ok') is not True:
                raise ValueError('Public command returned failure')
            return envelope['data']

        if legacy_skill:
            # Install the real immutable 0.4.1 package, then let the old bootstrap/updater upgrade it.
            old_index = download('releases/0.4.1/release.json', 65536)
            if hashlib.sha256(old_index).hexdigest() != '3ac7ef777c4f87b9a748755890eec675dd1223e6b8f4de35ed74edda285e005a':
                raise ValueError('Legacy release index differs from the verified historical version')
            architecture = {'aarch64':'arm64','amd64':'x86_64'}.get(platform.machine().lower(), platform.machine().lower())
            old = next(p for p in json.loads(old_index)['packages'] if (p['os'],p['architecture']) == (platform.system(),architecture))
            old_zip = temporary / 'old-cli.zip'
            old_bytes = download(old['key'], old['bytes'])
            if len(old_bytes) != old['bytes'] or hashlib.sha256(old_bytes).hexdigest() != old['sha256']:
                raise ValueError('Legacy native package digest differs')
            old_zip.write_bytes(old_bytes)
            binary = temporary / ('dt-cli.exe' if platform.system() == 'Windows' else 'dt-cli')
            with zipfile.ZipFile(old_zip) as package:
                binary.write_bytes(package.read(binary.name))
            binary.chmod(0o755)
            run([binary, 'install', '--package', old_zip, '--sha256', old['sha256'], '--directory', installation])
        first = run(argv)
        if first['action'] != ('upgrade' if legacy_skill else 'install') or first['version'] != release['version']:
            raise ValueError('Public first installation differs')
        probe = run([first['launcher'], 'version'])
        if probe['buildCommit'] != commit or probe['catalogDigest'] != release['catalogDigest']:
            raise ValueError('Installed public payload provenance differs')
        if legacy_skill:
            old_cache = json.loads((installation / 'distribution-cache.json').read_bytes())
            if 'channelKey' in old_cache:
                raise ValueError('Expected a cache written by the original CLI updater')
            migrated = run([first['launcher'], 'upgrade', '--online'])
            if migrated['changed'] is not False or json.loads((installation / 'distribution-cache.json').read_bytes())['channelKey'] != config['channelKey']:
                raise ValueError('Legacy update cache did not migrate to the native channel')
        active = (installation / 'active').read_bytes()
        repeated = run(argv)
        if repeated['updateCheck'] != 'cached' or (installation / 'active').read_bytes() != active:
            raise ValueError('Repeated public bootstrap must use the verified cache without switching')
        checked = run([first['launcher'], 'upgrade', '--online', '--check'])
        if checked['updateAvailable'] is not False or checked['changed'] is not False or (installation / 'active').read_bytes() != active:
            raise ValueError('Actual public online check differs or changed installation')
    print(json.dumps(dict(publicPrefix=base, version=release['version'], skillVersion=installed_config['skillVersion'], buildCommit=commit, legacyUpgradeVerified=legacy_skill,
                         os=platform.system(), checks=(['public-stable-skill-digest'] if skill_only else []) + ['public-skill-and-index-digests', 'public-first-install',
                         'public-launcher-provenance', 'public-cached-repeat', 'public-online-check'], iamLoginPerformed=False)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--skill-only', action='store_true')
    modes.add_argument('--legacy-skill', action='store_true')
    args = parser.parse_args()
    verify(args.skill_only, args.legacy_skill)
