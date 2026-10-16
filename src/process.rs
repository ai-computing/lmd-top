//! Bounded subprocess execution shared by collectors and operation workers.
use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Commands own a process group on Unix so cancellation includes descendants.
#[cfg(unix)]
struct ProcessGroup(Option<i32>);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0.take().filter(|pid| *pid > 1) {
            // SAFETY: the positive PGID came from our newly spawned child with
            // process_group(0); negative PGID targets that isolated group only.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
}

pub async fn run(mut cmd: Command, input: Option<Vec<u8>>, budget: Duration) -> Result<Output> {
    cmd.kill_on_drop(true)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().process_group(0);
    }
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!("{program} not found in PATH — install it or fix PATH")
        } else {
            anyhow!("cannot start {program}: {e}")
        }
    })?;
    #[cfg(unix)]
    let mut group = ProcessGroup(child.id().and_then(|pid| i32::try_from(pid).ok()));
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // Read both pipes while writing stdin, avoiding pipe-capacity deadlocks.
    let io = async {
        async fn read(pipe: Option<impl tokio::io::AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
            use tokio::io::AsyncReadExt;
            let mut bytes = Vec::new();
            if let Some(pipe) = pipe {
                pipe.take((MAX_OUTPUT_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await?;
                if bytes.len() > MAX_OUTPUT_BYTES {
                    return Err(std::io::Error::other(format!(
                        "subprocess output exceeded {MAX_OUTPUT_BYTES} bytes per stream"
                    )));
                }
            }
            Ok(bytes)
        }
        let write = async {
            if let (Some(stdin), Some(input)) = (stdin.as_mut(), input) {
                stdin.write_all(&input).await?;
                stdin.shutdown().await?;
            }
            // Close the actual pipe: shutdown alone need not drop the handle.
            drop(stdin.take());
            Ok::<(), std::io::Error>(())
        };
        let (_, stdout, stderr, status) =
            tokio::try_join!(write, read(stdout), read(stderr), child.wait())?;
        Ok::<_, std::io::Error>(Output {
            status,
            stdout,
            stderr,
        })
    };
    match tokio::time::timeout(budget, io).await {
        Ok(Ok(output)) => {
            #[cfg(unix)]
            {
                group.0 = None;
            }
            Ok(output)
        }
        result => {
            // Kill descendants before reaping the direct child. The guard also
            // runs when the enclosing future is cancelled.
            #[cfg(unix)]
            drop(group);
            let _ = child.kill().await;
            match result {
                Ok(Err(e)) => Err(e.into()),
                _ => Err(anyhow!(
                    "subprocess timed out after {}s",
                    budget.as_secs_f64()
                )),
            }
        }
    }
}

/// Synchronous facade for the TUI's worker threads and headless commands. The dedicated
/// runtime thread also permits use from an async CLI without nesting Tokio runtimes.
pub struct BlockingCommand {
    command: Option<Command>,
    input: Option<Vec<u8>>,
    budget: Duration,
}
impl BlockingCommand {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            command: Some(Command::new(program)),
            input: None,
            budget: Duration::from_secs(25),
        }
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.as_mut().unwrap().args(args);
        self
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.as_mut().unwrap().arg(arg);
        self
    }
    pub fn input(&mut self, input: &[u8]) -> &mut Self {
        self.input = Some(input.to_vec());
        self
    }
    pub fn budget(&mut self, budget: Duration) -> &mut Self {
        self.budget = budget;
        self
    }
    pub fn output(&mut self) -> Result<Output> {
        let cmd = self
            .command
            .take()
            .ok_or_else(|| anyhow!("command already executed"))?;
        let input = self.input.take();
        let budget = self.budget;
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(run(cmd, input, budget))
        })
        .join()
        .map_err(|_| anyhow!("subprocess worker panicked"))?
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn oversized_stdout_or_stderr_is_an_error_not_truncated_success() {
        for redirect in ["", " >&2"] {
            let script = format!("head -c {} /dev/zero{redirect}", MAX_OUTPUT_BYTES + 1);
            let error = BlockingCommand::new("sh")
                .args(["-c", &script])
                .output()
                .unwrap_err();
            assert!(error.to_string().contains("output exceeded"));
        }
    }

    struct DescendantFixture(std::path::PathBuf);
    impl Drop for DescendantFixture {
        fn drop(&mut self) {
            // Even a failed assertion must not leave the test's shell group alive.
            if let Ok(pid) = std::fs::read_to_string(self.0.join("group")) {
                if let Ok(pid) = pid.trim().parse::<i32>() {
                    if pid > 1 {
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn descendant_shutdown(cancel: bool) {
        let root = std::env::temp_dir().join(format!(
            "lmd-process-handshake-{}-{}",
            std::process::id(),
            if cancel { "cancel" } else { "timeout" }
        ));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = DescendantFixture(root.clone());
        for name in ["group", "ready", "release", "escaped"] {
            let _ = std::fs::remove_file(root.join(name));
        }
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                r#"
            printf '%s' "$$" > "$1/group"
            (printf ready > "$1/ready"
             while [ ! -f "$1/release" ]; do sleep 0.02; done
             printf escaped > "$1/escaped") & wait
        "#,
                "test",
            ])
            .arg(&root);
        let task = tokio::spawn(run(command, None, Duration::from_secs(2)));
        let started = tokio::time::timeout(Duration::from_secs(1), async {
            while !root.join("ready").exists() {
                assert!(
                    !task.is_finished(),
                    "runner finished before descendant started"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(started.is_ok(), "descendant never acknowledged startup");
        assert!(!root.join("escaped").exists());
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert!(task
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("timed out"));
        }
        // Release only after confirmed cancellation/timeout. A surviving descendant
        // would now produce the marker; early process startup is no longer assumed.
        std::fs::write(root.join("release"), b"go").unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !root.join("escaped").exists(),
            "descendant survived runner shutdown"
        );
        drop(fixture);
    }

    #[test]
    fn cancelling_runner_stops_background_descendants() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(descendant_shutdown(true));
    }

    #[test]
    fn timeout_stops_background_descendants() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(descendant_shutdown(false));
    }

    #[test]
    fn captures_output_and_closes_stdin() {
        let out = BlockingCommand::new("sh")
            .args(["-c", "cat; printf err >&2; exit 7"])
            .input(b"hello")
            .output()
            .unwrap();
        assert_eq!(out.stdout, b"hello");
        assert_eq!(out.stderr, b"err");
        assert_eq!(out.status.code(), Some(7));
    }
    #[test]
    fn timeout_kills_and_reaps_child() {
        let start = std::time::Instant::now();
        let err = BlockingCommand::new("sh")
            .args(["-c", "exec sleep 10"])
            .budget(Duration::from_millis(100))
            .output()
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
