use super::*;

pub(super) struct SystemVerifier;
/// Fixed commands, bounded output and deadline. Paths/arguments never become shell source.
pub(super) fn run(command: &mut Command) -> Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| failure("PACKAGE_VERIFICATION_FAILED", "程序版本核验无法启动。"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let out = read(Box::new(stdout));
    let err = read(Box::new(stderr));
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = out
        .join()
        .map_err(|_| bad_package())?
        .map_err(|_| bad_package())?;
    let err = err
        .join()
        .map_err(|_| bad_package())?
        .map_err(|_| bad_package())?;
    if !status.is_some_and(|s| s.success()) || out.len() > 64 * 1024 || err.len() > 64 * 1024 {
        return Err(failure(
            "PACKAGE_VERIFICATION_FAILED",
            "程序版本核验未通过；当前安装未改变。",
        ));
    }
    let mut both = out;
    both.extend(err);
    Ok(both)
}
impl Verifier for SystemVerifier {
    fn probe(&self, binary: &Path) -> Result<Value> {
        serde_json::from_slice(&run(Command::new(binary).arg("version"))?)
            .map_err(|_| bad_package())
    }
}
