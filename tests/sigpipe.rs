use std::io::Read;
use std::process::{Command, Stdio};

#[test]
fn closed_stdout_pipe_does_not_panic() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt"))
        .args(["completions", "bash"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn yt");

    let mut stdout = child.stdout.take().unwrap();
    let mut buf = [0u8; 8];
    stdout.read_exact(&mut buf).expect("read first bytes");
    drop(stdout);

    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert!(!stderr.contains("Broken pipe"), "stderr: {stderr}");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert!(
            output.status.code() == Some(0) || output.status.signal() == Some(libc::SIGPIPE),
            "status: {:?}",
            output.status
        );
    }
}
