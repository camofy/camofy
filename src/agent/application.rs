//! Bounded, credential-free application diagnostics and persistent retry policy.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, process::ExitStatus};
use tokio::io::{AsyncRead, AsyncReadExt};

const OUTPUT_LIMIT: usize = 32 * 1024;
const MAX_FAILURES: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Diagnostic {
    pub stage: String,
    pub kind: String,
    pub message: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub output_truncated: bool,
}
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.stage, self.message)?;
        if let Some(code) = self.exit_code {
            write!(f, " (exit {code})")?;
        }
        if let Some(signal) = self.signal {
            write!(f, " (signal {signal})")?;
        }
        Ok(())
    }
}
impl std::error::Error for Diagnostic {}
impl Diagnostic {
    pub fn new(stage: &str, kind: &str, message: &str) -> Self {
        Self {
            stage: stage.into(),
            kind: kind.into(),
            message: message.into(),
            exit_code: None,
            signal: None,
            output_truncated: false,
        }
    }
    pub fn safe(stage: &str, error: &anyhow::Error) -> Self {
        if let Some(diagnostic) = error.downcast_ref::<Self>() {
            return diagnostic.clone();
        }
        let text = format!("{error:#}");
        let (kind, message) = classify(text.as_bytes());
        Self::new(stage, kind, message)
    }
    pub fn rejected(status: ExitStatus, stdout: &Captured, stderr: &Captured) -> Self {
        let mut result = Self::new(
            "validation",
            "invalid_configuration",
            "Mihomo rejected the candidate configuration",
        );
        let joined = [stdout.bytes.as_slice(), stderr.bytes.as_slice()].concat();
        let (kind, message) = classify(&joined);
        result.kind = kind.into();
        result.message = message.into();
        if stdout.out_of_memory || stderr.out_of_memory {
            result.kind = "out_of_memory".into();
            result.message =
                "Mihomo could not allocate memory while validating the candidate configuration"
                    .into();
        }
        result.exit_code = status.code();
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            result.signal = status.signal();
        }
        result.output_truncated = stdout.truncated || stderr.truncated;
        result
    }
}

// Never return arbitrary program output: configuration lines, URLs, node names and
// panic stacks can contain subscription credentials. These reason strings are fixed.
fn classify(bytes: &[u8]) -> (&'static str, &'static str) {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if [
        "out of memory",
        "cannot allocate memory",
        "runtime: memory allocation",
    ]
    .iter()
    .any(|s| text.contains(s))
    {
        (
            "out_of_memory",
            "Mihomo could not allocate memory while processing configuration",
        )
    } else if text.contains("address already in use") {
        (
            "address_in_use",
            "A configured listening port is already in use",
        )
    } else if text.contains("no such file") || text.contains("not found (os error 2)") {
        (
            "missing_file",
            "A required local configuration, core or rule file is missing",
        )
    } else if text.contains("permission denied") {
        (
            "permission_denied",
            "The agent or Mihomo cannot access a required file or socket",
        )
    } else if text.contains("duplicate") || text.contains("already defined") {
        (
            "duplicate_definition",
            "The candidate contains duplicate configuration definitions",
        )
    } else if text.contains("proxy")
        && (text.contains("not found") || text.contains("does not exist"))
    {
        (
            "unknown_proxy",
            "The candidate references an unknown proxy or proxy group",
        )
    } else if text.contains("unsupported") || text.contains("unknown rule type") {
        (
            "unsupported_configuration",
            "The installed Mihomo does not support part of the candidate configuration",
        )
    } else if text.contains("yaml:") || text.contains("unmarshal") || text.contains("parse config")
    {
        (
            "invalid_configuration",
            "The candidate configuration could not be parsed or validated",
        )
    } else if text.contains("timed out") || text.contains("timeout") {
        (
            "timeout",
            "Configuration processing exceeded its time limit",
        )
    } else if text.contains("hash mismatch") {
        (
            "hash_mismatch",
            "The downloaded candidate does not match its expected hash",
        )
    } else {
        (
            "application_failed",
            "Configuration processing failed; no credential-safe error detail was available",
        )
    }
}

#[derive(Default)]
pub(super) struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
    out_of_memory: bool,
}
impl Captured {
    fn push(&mut self, bytes: &[u8]) {
        // Scan overlapping chunks so a split OOM message is still recognized, even
        // when later stack output pushes it outside the retained tail.
        let start = self.bytes.len().saturating_sub(128);
        let scan = [&self.bytes[start..], bytes].concat();
        self.out_of_memory |= classify(&scan).0 == "out_of_memory";
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > OUTPUT_LIMIT {
            self.bytes.drain(..self.bytes.len() - OUTPUT_LIMIT);
            self.truncated = true;
        }
    }
}
pub(super) async fn capture(mut stream: impl AsyncRead + Unpin) -> std::io::Result<Captured> {
    let mut result = Captured::default();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            return Ok(result);
        }
        result.push(&buffer[..count]);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Retry {
    pub failures: u32,
    pub next_retry_at: u64,
    pub delay_seconds: u64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Failure {
    revision: String,
    diagnostic: Diagnostic,
    retry: Retry,
}
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct State {
    pub attempted_revision: Option<String>,
    pub diagnostic: Option<Diagnostic>,
    pub retry: Option<Retry>,
    pub applied_inputs: Option<String>,
    pub pending_candidate: Option<(String, String, String)>,
    failures: BTreeMap<String, Failure>,
}
impl State {
    pub fn retry_due(&self, key: &str, now: u64) -> bool {
        self.failures
            .get(key)
            .is_none_or(|f| now >= f.retry.next_retry_at)
    }
    pub fn blocked(&mut self, key: &str, now: u64) -> Option<Diagnostic> {
        let failure = self.failures.get(key)?;
        if now >= failure.retry.next_retry_at {
            return None;
        }
        self.attempted_revision = Some(failure.revision.clone());
        self.diagnostic = Some(failure.diagnostic.clone());
        self.retry = Some(failure.retry.clone());
        self.diagnostic.clone()
    }
    pub fn failed(&mut self, key: String, revision: &str, diagnostic: Diagnostic, now: u64) {
        // A failed recovery invalidates the proof that these inputs are active.
        // Keeping this marker would let sync skip recovery and report "applied".
        if self.applied_inputs.as_deref() == Some(&key) {
            self.applied_inputs = None;
        }
        let failures = self
            .failures
            .get(&key)
            .map_or(1, |f| f.retry.failures.saturating_add(1));
        let delay_seconds =
            (30u64.saturating_mul(1u64 << failures.saturating_sub(1).min(6))).min(1800);
        let retry = Retry {
            failures,
            next_retry_at: now.saturating_add(delay_seconds),
            delay_seconds,
        };
        self.attempted_revision = Some(revision.into());
        self.diagnostic = Some(diagnostic.clone());
        self.retry = Some(retry.clone());
        self.failures.insert(
            key,
            Failure {
                revision: revision.into(),
                diagnostic,
                retry,
            },
        );
        if self.failures.len() > MAX_FAILURES
            && let Some(old) = self
                .failures
                .iter()
                .min_by_key(|(_, f)| f.retry.next_retry_at)
                .map(|(k, _)| k.clone())
        {
            self.failures.remove(&old);
        }
    }
    pub fn succeeded(&mut self, key: String, revision: &str) {
        self.failures.remove(&key);
        self.attempted_revision = Some(revision.into());
        self.diagnostic = None;
        self.retry = None;
        self.applied_inputs = Some(key);
    }
    pub fn restored(&mut self, key: String, revision: &str) {
        self.failures.remove(&key);
        self.applied_inputs = Some(key.clone());
        if self
            .pending_candidate
            .as_ref()
            .is_none_or(|(_, _, pending_key)| pending_key == &key)
        {
            self.succeeded(key, revision);
            self.pending_candidate = None;
        }
    }
    pub fn manual_retry(&mut self) {
        self.failures.clear();
        self.retry = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_is_bounded_and_oom_survives_truncation() {
        let mut capture = Captured::default();
        capture.push(b"password: very-secret\nfatal error: runtime: out of ");
        capture.push(b"memory\n");
        for _ in 0..100 {
            capture.push(&[b'x'; 4096]);
        }
        assert!(capture.out_of_memory);
        assert!(capture.truncated);
        assert_eq!(capture.bytes.len(), OUTPUT_LIMIT);
    }
    #[tokio::test]
    async fn capture_drains_large_output_without_pipe_deadlock() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(64);
        let task = tokio::spawn(async move {
            writer
                .write_all(b"fatal error: runtime: out of memory\npassword: sensitive\n")
                .await
                .unwrap();
            for _ in 0..100 {
                writer.write_all(&[b'x'; 4096]).await.unwrap();
            }
        });
        let captured = tokio::time::timeout(std::time::Duration::from_secs(5), capture(reader))
            .await
            .unwrap()
            .unwrap();
        task.await.unwrap();
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(2 << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(2)
        };
        let diagnostic = Diagnostic::rejected(status, &captured, &Default::default());
        assert_eq!(diagnostic.kind, "out_of_memory");
        assert_eq!(diagnostic.exit_code, Some(2));
        assert!(diagnostic.output_truncated);
        assert!(
            !serde_json::to_string(&diagnostic)
                .unwrap()
                .contains("sensitive")
        );
    }
    #[test]
    fn unknown_output_and_known_reason_never_leak_credentials() {
        for input in [
            "yaml: password: secret-value",
            "fatal error: out of memory; https://user:pass@host/sub/token",
            "unsupported rule token-secret",
            "secret: unknown-secret",
        ] {
            let diagnostic = Diagnostic::safe("validation", &anyhow::anyhow!("{input}"));
            let serialized = serde_json::to_string(&diagnostic).unwrap();
            assert!(!serialized.contains("secret-value"));
            assert!(!serialized.contains("user:pass"));
            assert!(!serialized.contains("token-secret"));
            assert!(!serialized.contains("unknown-secret"));
            assert!(serialized.len() < 500);
        }
    }
    #[test]
    fn retry_persists_caps_and_changed_inputs_or_manual_retry_bypass_it() {
        let mut state = State::default();
        for n in 1..20 {
            state.failed(
                "same".into(),
                "bad",
                Diagnostic::new("validation", "out_of_memory", "memory allocation failed"),
                1000,
            );
            assert_eq!(state.retry.as_ref().unwrap().failures, n);
            assert!(state.retry.as_ref().unwrap().delay_seconds <= 1800);
        }
        let mut restored: State =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(restored.blocked("same", 1001).is_some());
        assert!(
            restored
                .blocked("changed-overlay-or-revision", 1001)
                .is_none()
        );
        assert!(restored.blocked("same", 2800).is_none());
        restored.succeeded("last-good".into(), "good");
        assert!(
            restored.blocked("same", 1001).is_some(),
            "recovering last good must not erase candidate cooldown"
        );
        restored.manual_retry();
        assert!(restored.blocked("same", 1001).is_none());
    }
    #[test]
    fn failed_recovery_invalidates_active_inputs_and_restore_clears_only_its_failure() {
        let mut state = State::default();
        state.succeeded("good".into(), "A");
        state.failed(
            "good".into(),
            "A",
            Diagnostic::new("health", "core_unavailable", "unavailable"),
            1000,
        );
        assert!(state.applied_inputs.is_none());
        state.restored("good".into(), "A");
        assert!(state.diagnostic.is_none());
        assert!(state.retry.is_none());
        assert_eq!(state.applied_inputs.as_deref(), Some("good"));
        state.failed(
            "bad".into(),
            "B",
            Diagnostic::new("validation", "out_of_memory", "allocation failed"),
            1000,
        );
        state.pending_candidate = Some(("B".into(), "hash-B".into(), "bad".into()));
        state.restored("good".into(), "A");
        assert_eq!(state.attempted_revision.as_deref(), Some("B"));
        assert!(state.blocked("bad", 1001).is_some());
    }
}
