"""Current release acceptance works after intentional deletion of historical packages."""
import contextlib
import hashlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
from types import SimpleNamespace
import unittest
import urllib.error
import zipfile
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts/release'))
SPEC = importlib.util.spec_from_file_location('public_verification', ROOT / 'scripts/release/verify_public.py')
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class PublicVerificationTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.config = json.loads((ROOT / 'skills/dt-cli/scripts/distribution.json').read_bytes())
        self.commit = 'a' * 40
        self.requested = []
        config_path = self.root / 'skills/dt-cli/scripts/distribution.json'
        config_path.parent.mkdir(parents=True)
        config_bytes = json.dumps(self.config).encode()
        config_path.write_bytes(config_bytes)
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, 'w') as package:
            package.writestr('dt-cli/scripts/distribution.json', config_bytes)
        skill_bytes = archive.getvalue()
        skill = {'key': f"skills/{self.config['skillVersion']}/dt-cli-skill.zip",
                 'bytes': len(skill_bytes), 'sha256': hashlib.sha256(skill_bytes).hexdigest()}
        release_key = 'releases/0.7.0/native-release.json'
        release = json.dumps({'buildCommit': self.commit, 'version': '0.7.0',
                              'catalogDigest': 'fixture-digest', 'skills': [skill]}).encode()
        self.objects = {
            self.config['channelKey']: json.dumps({
                'releaseKey': release_key, 'version': '0.7.0',
                'releaseSha256': hashlib.sha256(release).hexdigest()}).encode(),
            release_key: release, skill['key']: skill_bytes,
        }

    def open(self, request, **kwargs):
        key = request.full_url.removeprefix(self.config['publicBaseUrl'])
        self.requested.append(key)
        if key not in self.objects:
            raise urllib.error.HTTPError(request.full_url, 404, 'Not Found', {}, None)
        return io.BytesIO(self.objects[key])

    def process(self, args, **kwargs):
        args = list(map(str, args))
        if args[-1] == 'version':
            data = {'buildCommit': self.commit, 'catalogDigest': 'fixture-digest'}
        elif args[1].endswith('bootstrap.sh'):
            installation = pathlib.Path(args[-1])
            existing = installation.exists()
            installation.mkdir(exist_ok=True)
            (installation / 'active').write_bytes(b'current-release')
            data = {'action': 'existing' if existing else 'install', 'version': '0.7.0',
                    'launcher': str(installation / 'dt-cli'), 'updateCheck': 'cached'}
        elif args[1].endswith('install-skill.sh'):
            destination = pathlib.Path(args[args.index('--directory') + 1])
            destination.mkdir()
            (destination / 'LICENSE').write_text('original')
            data = {'version': self.config['skillVersion'], 'action': 'install'}
        elif args[1] == 'skill':
            destination = pathlib.Path(args[args.index('--directory') + 1])
            if (destination / 'LICENSE').read_text() == 'local modification':
                return SimpleNamespace(returncode=1, stdout=json.dumps({
                    'ok': False, 'error': {'code': 'SKILL_LOCAL_CHANGED'}}), stderr='')
            data = {'action': 'existing', 'changed': False}
        elif args[1] == 'upgrade':
            data = {'updateAvailable': False, 'changed': False}
        else:
            self.fail(f'Unexpected subprocess: {args}')
        return SimpleNamespace(returncode=0, stdout=json.dumps({'ok': True, 'data': data}), stderr='')

    def verify(self):
        output = io.StringIO()
        with patch.object(VERIFY, 'ROOT', self.root), \
             patch.object(VERIFY.urllib.request, 'build_opener', return_value=self), \
             patch.object(VERIFY.subprocess, 'run', side_effect=self.process), \
             patch.object(VERIFY.platform, 'system', return_value='Darwin'), \
             patch.object(VERIFY.time, 'sleep'), contextlib.redirect_stdout(output):
            VERIFY.verify(expected_commit=self.commit)
        return json.loads(output.getvalue())

    def test_default_accepts_current_release_without_historical_objects(self):
        result = self.verify()
        self.assertEqual(result['version'], '0.7.0')
        self.assertNotIn('old-cli-0.5.5-upgrade', result['checks'])
        self.assertFalse(any('/0.5.5/' in key or '/0.4.1/' in key for key in self.requested))

    def test_current_package_digest_is_still_required_without_historical_checks(self):
        key = next(key for key in self.objects if key.endswith('.zip'))
        self.objects[key] = b'x' * len(self.objects[key])
        with self.assertRaisesRegex(ValueError, 'Published Skill digest differs'):
            self.verify()


if __name__ == '__main__':
    unittest.main()
