"""Exercise shipped bootstrap scripts with an isolated local HTTPS distribution and native CLI."""
import hashlib
import http.server
import json
import os
import pathlib
import platform
import shutil
import ssl
import subprocess
import tempfile
import threading
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
NATIVE = os.environ.get('DT_CLI_NATIVE_BOOTSTRAP_BINARY')


class SilentFiles(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


@unittest.skipUnless(NATIVE, 'requires the final native release executable in CI')
class NativeBootstrapTest(unittest.TestCase):
    def run_command(self, argv, **kwargs):
        return subprocess.run([str(a) for a in argv], capture_output=True, text=True,
                              encoding='utf-8', timeout=180, **kwargs)

    def test_first_install_repeat_offline_and_bad_zip_keep_installed_program(self):
        binary = pathlib.Path(NATIVE).resolve()
        probe = json.loads(self.run_command([binary, 'version']).stdout)['data']
        with tempfile.TemporaryDirectory(prefix='dt-cli-https-') as temporary:
            root = pathlib.Path(temporary).resolve()
            scripts = root / 'skill scripts'
            shutil.copytree(ROOT / 'skills/dt-cli/scripts', scripts)
            files = root / 'public'
            files.mkdir()
            archive = files / 'releases' / probe['cliVersion'] / ('dt-cli-windows-x64.zip' if platform.system() == 'Windows' else 'dt-cli-macos-arm64.zip')
            archive.parent.mkdir(parents=True)
            built = self.run_command([shutil.which('python3') or shutil.which('python'), ROOT / 'scripts/release/package.py', '--binary', binary, '--output', archive])
            self.assertEqual(built.returncode, 0, built.stderr)
            manifest = json.loads(archive.with_suffix('.manifest.json').read_bytes())
            selected = dict(target=manifest['buildTarget'], os=manifest['os'], architecture=manifest['architecture'],
                            key=archive.relative_to(files).as_posix(), sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),
                            bytes=archive.stat().st_size, binarySha256=manifest['sha256'])
            other = dict(selected)
            if platform.system() == 'Darwin':
                other.update(target='x86_64-pc-windows-msvc', os='Windows', architecture='x86_64')
            else:
                other.update(target='aarch64-apple-darwin', os='Darwin', architecture='arm64')
            other['key'] = f'releases/{probe["cliVersion"]}/unused.zip'
            release = dict(schemaVersion=1, version=probe['cliVersion'], buildCommit=probe['buildCommit'],
                           catalogDigest=probe['catalogDigest'], compatibility=dict(bootstrapSchema=1, profileFormat=1,
                           credentialFormat=1, installerSchema=1, launcherSchema=1, minimumSkillVersion='0.4.1',
                           maximumSkillVersionExclusive='0.5.0'), packages=[selected, other], skills=[])
            release_path = archive.parent / 'release.json'
            release_path.write_text(json.dumps(release))
            (files / 'channels').mkdir()
            (files / 'channels/stable.json').write_text(json.dumps(dict(schemaVersion=1, sequence=1, version=probe['cliVersion'],
                releaseKey=release_path.relative_to(files).as_posix(), releaseSha256=hashlib.sha256(release_path.read_bytes()).hexdigest())))

            # Generate a disposable localhost certificate, never disable TLS verification.
            openssl = shutil.which('openssl')
            if not openssl and platform.system() == 'Windows':
                openssl = r'C:\Program Files\Git\usr\bin\openssl.exe'
            certificate, private_key = root / 'certificate.pem', root / 'private.pem'
            generated = self.run_command([openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-sha256', '-days', '1', '-nodes',
                '-keyout', private_key, '-out', certificate, '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost'])
            self.assertEqual(generated.returncode, 0, generated.stderr)
            handler = lambda *args, **kwargs: SilentFiles(*args, directory=str(files), **kwargs)
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
            tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            tls.load_cert_chain(certificate, private_key)
            server.socket = tls.wrap_socket(server.socket, server_side=True)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            config = json.loads((scripts / 'distribution.json').read_bytes())
            config['publicBaseUrl'] = f'https://localhost:{server.server_port}/'
            (scripts / 'distribution.json').write_text(json.dumps(config))
            env = os.environ.copy()
            env['CURL_CA_BUNDLE'] = str(certificate)
            powershell = shutil.which('pwsh') or shutil.which('powershell')
            fingerprint = None
            if platform.system() == 'Windows':
                der_certificate = root / 'certificate.cer'
                converted = self.run_command([openssl, 'x509', '-in', certificate, '-outform', 'DER', '-out', der_certificate])
                self.assertEqual(converted.returncode, 0, converted.stderr)
                trust_script = root / 'trust-fixture.ps1'
                trust_script.write_text(
                    'param([string]$Path)\n$certificate=New-Object Security.Cryptography.X509Certificates.X509Certificate2($Path); '
                    '$store=New-Object Security.Cryptography.X509Certificates.X509Store("Root","CurrentUser"); '
                    '$store.Open("ReadWrite"); $store.Add($certificate); $store.Close(); $certificate.Thumbprint')
                trust = self.run_command([powershell, '-NoProfile', '-File', trust_script, '-Path', der_certificate])
                self.assertEqual(trust.returncode, 0, trust.stderr)
                fingerprint = trust.stdout.strip()
            try:
                installation = root / 'installation 中文 with spaces'
                if platform.system() == 'Darwin':
                    argv = ['bash', scripts / 'bootstrap.sh', '--directory', installation]
                else:
                    argv = [powershell, '-NoProfile', '-File', scripts / 'bootstrap.ps1', '-Directory', installation]
                first = self.run_command(argv, env=env)
                self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
                first_data = json.loads(first.stdout)['data']
                self.assertEqual(first_data['action'], 'install')
                self.assertEqual(first_data['version'], probe['cliVersion'])
                launcher = pathlib.Path(first_data['launcher'])
                self.assertTrue(launcher.is_absolute())
                self.assertEqual(json.loads(self.run_command([launcher, 'version']).stdout)['data'], probe)
                active = (installation / 'active').read_bytes()
                archive.write_bytes(b'corrupt after installation')
                repeat = self.run_command(argv, env=env)
                self.assertEqual(repeat.returncode, 0, repeat.stdout + repeat.stderr)
                self.assertEqual((installation / 'active').read_bytes(), active)
                # With no installation, a broken package is rejected before executing payload.
                fresh = root / 'rejected installation'
                failed_argv = [fresh if a == installation else a for a in argv]
                failed = self.run_command(failed_argv, env=env)
                self.assertNotEqual(failed.returncode, 0)
                self.assertFalse(fresh.exists())
                server.shutdown()
                offline = self.run_command(argv, env=env)
                self.assertEqual(offline.returncode, 0, offline.stdout + offline.stderr)
                self.assertEqual((installation / 'active').read_bytes(), active)
            finally:
                server.shutdown()
                server.server_close()
                if fingerprint:
                    cleanup_script = root / 'cleanup-fixture.ps1'
                    cleanup_script.write_text('param([string]$Thumbprint)\nRemove-Item -LiteralPath ("Cert:\\CurrentUser\\Root\\" + $Thumbprint)')
                    removed = self.run_command([powershell, '-NoProfile', '-File', cleanup_script, '-Thumbprint', fingerprint])
                    self.assertEqual(removed.returncode, 0, removed.stderr)


if __name__ == '__main__':
    unittest.main()
