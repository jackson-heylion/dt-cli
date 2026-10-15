use super::*;

pub(super) struct SystemVerifier;
/// Fixed commands, bounded output and deadline. Paths/arguments never become shell source.
pub(super) fn run(command: &mut Command) -> Result<Vec<u8>> {
    run_bounded(
        command,
        Duration::from_secs(crate::constants::distribution::PROBE_SECONDS),
        crate::constants::distribution::PROBE_BYTES,
    )
}
pub(super) fn run_bounded(command: &mut Command, limit: Duration, maximum: u64) -> Result<Vec<u8>> {
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
            pipe.take(maximum + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let out = read(Box::new(stdout));
    let err = read(Box::new(stderr));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(
                crate::constants::distribution::PROCESS_POLL_MILLIS,
            )),
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
    if !status.is_some_and(|s| s.success())
        || out.len() as u64 > maximum
        || err.len() as u64 > maximum
    {
        return Err(failure(
            "PACKAGE_VERIFICATION_FAILED",
            "程序版本核验未通过；当前安装未改变。",
        ));
    }
    // stderr is checked against the same output budget, but it is not part of the JSON protocol.
    Ok(out)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_output_does_not_change_json_stdout() {
        let output =
            run(Command::new("sh")
                .args(["-c", "printf '{\"ok\":true}'; printf 'diagnostic\n' >&2"]))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["ok"],
            true
        );
    }

    #[test]
    fn a_process_cannot_exceed_its_deadline_or_output_budget() {
        assert!(
            run_bounded(
                Command::new("sh").args(["-c", "while :; do :; done"]),
                Duration::from_millis(30),
                32,
            )
            .is_err()
        );
        assert!(
            run_bounded(
                Command::new("sh").args(["-c", "printf '0123456789'"]),
                Duration::from_secs(1),
                4,
            )
            .is_err()
        );
    }
}
impl Verifier for SystemVerifier {
    fn probe(&self, binary: &Path) -> Result<Value> {
        serde_json::from_slice(&run(Command::new(binary).arg("version"))?)
            .map_err(|_| bad_package())
    }
}
