//! A live core owns network cleanup. Never use kill-on-drop or SIGKILL here.
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::process::Child;

pub(super) async fn terminate(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    let pid = child.id().context("core PID unavailable")?;
    #[cfg(unix)]
    {
        // The child handle has not been reaped, so this PID cannot be reused.
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
    }
    #[cfg(windows)]
    {
        // No /F: unsupported graceful console termination must fail visibly.
        let status = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;
        anyhow::ensure!(
            status.success() || child.try_wait()?.is_some(),
            "core refused graceful termination"
        );
    }
    tokio::time::timeout(Duration::from_secs(30), child.wait())
        .await
        .context("Mihomo has not exited after SIGTERM; no force-kill attempted")??;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    #[tokio::test]
    async fn termination_runs_cleanup_handler_before_returning() {
        let dir = std::env::temp_dir().join(format!("camofy-shutdown-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut child = tokio::process::Command::new("sh")
            .arg("-c").arg("trap 'echo cleaned > cleanup; exit 0' TERM; echo ready > ready; while :; do sleep 0.1; done")
            .current_dir(&dir).spawn().unwrap();
        for _ in 0..100 {
            if dir.join("ready").exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(dir.join("ready").exists());
        super::terminate(&mut child).await.unwrap();
        assert!(dir.join("cleanup").exists());
        super::terminate(&mut child).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
