// JXA runs with the system osascript runtime; JSON is data, never executable code.
ObjC.import('Foundation');
function read(path) {
    var data = $.NSData.dataWithContentsOfFile(path);
    if (!data || Number(data.length) > 65536) throw Error('BOOTSTRAP_METADATA_INVALID');
    return JSON.parse(ObjC.unwrap($.NSString.alloc.initWithDataEncoding(data, $.NSUTF8StringEncoding)));
}
function requireValue(condition) { if (!condition) throw Error('BOOTSTRAP_METADATA_INVALID'); }
function version(value) {
    requireValue(typeof value === 'string' && /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(value));
    return value.split('.').map(Number);
}
function compare(a, b) {
    a = version(a); b = version(b);
    for (var i = 0; i < 3; i++) { if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1; }
    return 0;
}
function key(value) {
    requireValue(typeof value === 'string' && value.length < 200 && /^(releases|skills)\/[a-zA-Z0-9./_-]+$/.test(value));
    requireValue(value.split('/').every(function (v) { return v && v !== '.' && v !== '..'; }));
}
function hex(value, n) { return typeof value === 'string' && new RegExp('^[0-9a-f]{' + n + '}$').test(value); }
function config(value) {
    requireValue(value.schemaVersion === 1 && value.bootstrapSchema === 1 && value.channelKey === 'channels/native-stable.json');
    version(value.minimumCliVersion); version(value.skillVersion);
    if (!/^https:\/\/[a-zA-Z0-9.-]+(?::[0-9]+)?\/(?:[a-zA-Z0-9_-]+\/)*$/.test(value.publicBaseUrl)) {
        throw Error('DISTRIBUTION_NOT_CONFIGURED');
    }
    return value;
}
function run(argv) {
    var mode = argv[0];
    if (mode === 'config') return JSON.stringify(config(read(argv[1])));
    if (mode === 'plan') {
        var source = config(read(argv[1])), stable = read(argv[2]), release = read(argv[3]);
        requireValue(stable.schemaVersion === 1 && stable.sequence > 0 && Math.floor(stable.sequence) === stable.sequence && stable.sequence <= 9007199254740991);
        requireValue(stable.releaseKey === 'releases/' + stable.version + '/native-release.json' && hex(stable.releaseSha256, 64));
        requireValue(release.schemaVersion === 1 && release.version === stable.version && compare(release.version, source.minimumCliVersion) >= 0);
        requireValue(hex(release.buildCommit, 40) && hex(release.catalogDigest, 64));
        var c = release.compatibility;
        ['bootstrapSchema', 'profileFormat', 'credentialFormat', 'installerSchema', 'launcherSchema'].forEach(function (k) { requireValue(c[k] === 1); });
        requireValue(compare(source.skillVersion, c.minimumSkillVersion) >= 0 && compare(source.skillVersion, c.maximumSkillVersionExclusive) < 0);
        requireValue(release.packages.length === 3 && ['arm64', 'x86_64'].indexOf(argv[4]) >= 0);
        var selected;
        [['aarch64-apple-darwin', 'Darwin', 'arm64'], ['x86_64-apple-darwin', 'Darwin', 'x86_64'], ['x86_64-pc-windows-msvc', 'Windows', 'x86_64']].forEach(function (expected) {
            var matches = release.packages.filter(function (p) { return p.target === expected[0]; });
            requireValue(matches.length === 1);
            var p = matches[0]; key(p.key);
            requireValue(p.os === expected[1] && p.architecture === expected[2] && p.key.indexOf('releases/' + release.version + '/') === 0);
            requireValue(hex(p.sha256, 64) && hex(p.binarySha256, 64) && p.bytes > 0 && p.bytes <= 268435456 && Math.floor(p.bytes) === p.bytes);
            if (p.os === 'Darwin' && p.architecture === argv[4]) selected = p;
        });
        return JSON.stringify({version:release.version, buildCommit:release.buildCommit, catalogDigest:release.catalogDigest, package:selected});
    }
    if (mode === 'manifest') {
        var m = read(argv[1]), plan = read(argv[2]);
        requireValue(m.manifestSchemaVersion === 1 && m.releaseType === 'release' && m.localDevelopment === false && m.nativeProbe === 'performed');
        requireValue(m.os === 'Darwin' && m.architecture === plan.package.architecture && m.buildTarget === plan.package.target && m.binary === 'dt-cli');
        ['profileFormat', 'credentialFormat', 'minimumInstallerSchema', 'minimumLauncherSchema'].forEach(function (k) { requireValue(m[k] === 1); });
        requireValue(hex(m.sha256, 64) && hex(m.buildCommit, 40) && hex(m.catalogDigest, 64));
        requireValue(m.version === plan.version && m.buildCommit === plan.buildCommit && m.catalogDigest === plan.catalogDigest && m.sha256 === plan.package.binarySha256);
        return m.sha256;
    }
    if (mode === 'probe') {
        var body = read(argv[1]);
        requireValue(['arm64', 'x86_64'].indexOf(argv[3]) >= 0);
        var target = argv[3] === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
        requireValue(body.ok === true && body.data.localDevelopment === false && body.data.buildTarget === target);
        requireValue(compare(body.data.cliVersion, argv[2]) >= 0);
        return body.data.cliVersion;
    }
    if (mode === 'result') {
        var probe = read(argv[1]);
        return JSON.stringify({ok:true, data:{launcher:argv[2], version:probe.data.cliVersion, action:argv[3], updateCheck:argv[4]}, error:null});
    }
    throw Error('BOOTSTRAP_ARGUMENT_INVALID');
}
