"""Internal filesystem upgrade/rollback with checksums and native version probes."""
import contextlib,hashlib,io,json,pathlib,runpy,subprocess,sys,tempfile,unittest,zipfile
from unittest.mock import patch
SCRIPT=pathlib.Path(__file__).resolve().parents[2]/'scripts/release/install.py'
class InstallTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=pathlib.Path(self.temp.name);self.dest=self.root/'dt-cli';self.dest.write_bytes(b'old')
        self.profile=self.root/'profile.json';self.profile.write_text('preserved')
        self.package=self.root/'release.zip'
    def archive(self,data=b'new',digest=None,system='Darwin',**fields):
        with zipfile.ZipFile(self.package,'w') as z:
            z.writestr('dt-cli',data)
            z.writestr('manifest.json',json.dumps({'binary':'dt-cli','version':'0.2.0','sha256':digest or hashlib.sha256(data).hexdigest(),'os':system,'architecture':'arm64',**fields}))
    def run_install(self,*extra,fail_probe=False,probe_data=None,expected='auto'):
        def command(args,**kwargs):
            if args[-1]!='version':raise AssertionError('Internal install must not invoke signature tools')
            if fail_probe:raise subprocess.CalledProcessError(1,args)
            return subprocess.CompletedProcess(args,0,json.dumps({'ok':True,'data':probe_data or {'cliVersion':'0.2.0'}}),'')
        argv=[str(SCRIPT),'--destination',str(self.dest),'--package',str(self.package),*extra]
        if expected=='auto':
            source=self.dest.with_name('dt-cli.previous') if '--rollback' in extra else self.package
            expected=hashlib.sha256(source.read_bytes()).hexdigest()
        if expected is not None:argv.extend(['--sha256',expected])
        with patch.object(sys,'argv',argv),patch('platform.system',return_value='Darwin'),patch('platform.machine',return_value='arm64'),patch('subprocess.run',side_effect=command),contextlib.redirect_stdout(io.StringIO()):
            runpy.run_path(str(SCRIPT),run_name='__main__')
    def test_upgrade_and_rollback_preserve_profile(self):
        self.archive();self.run_install()
        self.assertEqual(self.dest.read_bytes(),b'new');self.assertEqual(self.dest.with_name('dt-cli.previous').read_bytes(),b'old')
        self.run_install('--rollback')
        self.assertEqual(self.dest.read_bytes(),b'old');self.assertEqual(self.profile.read_text(),'preserved')
    def test_tampered_package_does_not_replace(self):
        self.archive(digest='0'*64)
        with self.assertRaises(SystemExit):self.run_install()
        self.assertEqual(self.dest.read_bytes(),b'old')
    def test_other_platform_does_not_replace(self):
        self.archive(system='Windows')
        with self.assertRaises(SystemExit):self.run_install()
        self.assertEqual(self.dest.read_bytes(),b'old')
    def test_first_install_requires_checksum_and_no_signer(self):
        self.dest.unlink();self.archive()
        with self.assertRaises(SystemExit):self.run_install(expected=None)
        self.assertFalse(self.dest.exists());self.run_install();self.assertEqual(self.dest.read_bytes(),b'new')
    def test_legacy_signer_argument_is_accepted_without_signature_tools(self):
        self.archive();self.run_install('--signer','OLD-TEAM')
        self.assertEqual(self.dest.read_bytes(),b'new')
    def test_wrong_external_archive_digest_cannot_replace(self):
        self.archive()
        with self.assertRaises(SystemExit):self.run_install(expected='0'*64)
        self.assertEqual(self.dest.read_bytes(),b'old')
    def test_unrunnable_binary_keeps_previous_executable(self):
        self.archive()
        prior=self.dest.with_name('dt-cli.previous');prior.write_bytes(b'prior-backup')
        with self.assertRaises(subprocess.CalledProcessError):self.run_install(fail_probe=True)
        self.assertEqual(prior.read_bytes(),b'prior-backup')
        self.assertEqual(self.dest.read_bytes(),b'old');self.assertEqual(self.profile.read_text(),'preserved')
    def test_rollback_without_current_binary_requires_backup_checksum(self):
        backup=self.dest.with_name('dt-cli.previous');backup.write_bytes(b'old');self.dest.unlink()
        with self.assertRaises(SystemExit):self.run_install('--rollback',expected=None)
        self.assertFalse(self.dest.exists());self.assertEqual(backup.read_bytes(),b'old')
        self.run_install('--rollback')
        self.assertEqual(self.dest.read_bytes(),b'old');self.assertEqual(self.profile.read_text(),'preserved')
    def test_new_release_provenance_matches_native_probe(self):
        fields={'buildCommit':'b'*40,'catalogVersion':'1.1.0','catalogDigest':'a'*64,'sourceVersion':'fixture'}
        self.archive(version='0.3.0',**fields)
        self.run_install(probe_data={'cliVersion':'0.3.0',**fields})
        self.assertEqual(self.dest.read_bytes(),b'new')
    def test_tampered_catalog_or_source_manifest_preserves_program_and_rollback(self):
        fields={'buildCommit':'b'*40,'catalogVersion':'1.1.0','catalogDigest':'a'*64,'sourceVersion':'fixture'}
        backup=self.dest.with_name('dt-cli.previous');backup.write_bytes(b'prior-backup')
        for field in fields:
            with self.subTest(field=field):
                changed={**fields,field:'tampered'};self.archive(version='0.3.0',**changed)
                with self.assertRaises(SystemExit):self.run_install(probe_data={'cliVersion':'0.3.0',**fields})
                self.assertEqual(self.dest.read_bytes(),b'old');self.assertEqual(backup.read_bytes(),b'prior-backup')
    def test_new_release_cannot_omit_provenance(self):
        self.archive(version='0.3.0')
        with self.assertRaises(SystemExit):self.run_install(probe_data={'cliVersion':'0.3.0'})
        self.assertEqual(self.dest.read_bytes(),b'old')
    def test_dirty_native_build_is_not_installable_as_a_new_release(self):
        fields={'buildCommit':'b'*40+'+dirty','catalogVersion':'1.1.0','catalogDigest':'a'*64,'sourceVersion':'fixture'}
        self.archive(version='0.3.0',**fields)
        with self.assertRaises(SystemExit):self.run_install(probe_data={'cliVersion':'0.3.0',**fields})
        self.assertEqual(self.dest.read_bytes(),b'old')
    def test_legacy_overwrite_tool_refuses_unimplemented_format_migration(self):
        self.archive(manifestSchemaVersion=1,profileFormat=2,credentialFormat=1,minimumInstallerSchema=1,minimumLauncherSchema=1,nativeProbe='performed')
        with self.assertRaises(SystemExit):self.run_install()
        self.assertEqual(self.dest.read_bytes(),b'old')
    def test_legacy_overwrite_tool_cannot_promote_a_candidate_package(self):
        self.archive(releaseType='candidate')
        backup=self.dest.with_name('dt-cli.previous');backup.write_bytes(b'preserved')
        with self.assertRaises(SystemExit):self.run_install()
        self.assertEqual(self.dest.read_bytes(),b'old');self.assertEqual(backup.read_bytes(),b'preserved')
    def test_native_probe_passes_special_path_as_one_argument(self):
        module=runpy.run_path(str(SCRIPT));calls=[]
        path=pathlib.Path("C:/发布 目录/dt-cli &('literal').exe")
        def command(args,**kwargs):
            calls.append(args);return subprocess.CompletedProcess(args,0,json.dumps({'ok':True,'data':{'cliVersion':'0.3.0'}}),'')
        with patch('subprocess.run',side_effect=command):
            self.assertEqual(module['probe'](path)['data']['cliVersion'],'0.3.0')
        self.assertEqual(calls,[[str(path),'version']])
if __name__=='__main__':unittest.main()
