//! Environment scrubbing for the external processes AMATL spawns.
//!
//! Every child process AMATL launches — Trafilatura, `pdftotext`, the Chromium
//! isolation harness — is asked to parse attacker-controlled bytes. A
//! memory-safety or deserialization bug in one of those parsers is therefore
//! remote code execution *inside the child*, and by default a child inherits
//! the parent's whole environment. The parent keeps the server bearer token
//! (`server.token_env`) and every provider credential (`credential_env`) in
//! that environment for its entire lifetime, so an unscrubbed child silently
//! hands the attacker the credentials that the HTTP layer spends its time
//! protecting.
//!
//! [`scrub_environment`] removes that inheritance: the child starts with an
//! empty environment and gets back only the non-secret variables it needs in
//! order to run at all. It is called at every spawn site in the crate, and
//! `extract.rs` proves the wiring end to end with a canary variable rather
//! than by convention.
//!
//! The module also owns the one Linux spawn failure that is not a real
//! failure: see [`spawn_with_busy_retry`].

use std::time::Duration;
use tokio::process::{Child, Command};

/// Non-secret variables a child may keep after [`scrub_environment`].
///
/// `PATH` is not optional: bare program names such as the default
/// `trafilatura` are resolved through it, and the `#!/usr/bin/env python3`
/// shebang of a pip-installed entry point needs it again for the interpreter
/// lookup. The rest is runtime context that changes how a program behaves and
/// never what it can steal: `HOME`/`TMPDIR` for scratch space,
/// `LANG`/`LC_ALL`/`TZ` for locale and time formatting.
///
/// Nothing may be added here that can hold a secret — that is the entire
/// point of the list. A name like `AMATL_SERVER_TOKEN` or a provider
/// `credential_env` belongs to the parent only.
pub const INHERITED_ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "TZ"];

/// Drop the inherited environment and restore only the allowlisted variables.
pub fn scrub_environment(command: &mut Command) {
    command.env_clear();
    forward_environment(command, INHERITED_ENV_ALLOWLIST);
}

/// Copy `names` from the parent environment into `command`, skipping the ones
/// the parent does not set.
///
/// Must be called *after* [`scrub_environment`], which discards everything set
/// before it. The renderer uses it for the handful of variables the isolation
/// harness legitimately needs — the `AMATL_CHROMIUM_*` overrides and the
/// systemd user-bus pair — and each one is named explicitly instead of being
/// inherited wholesale.
pub fn forward_environment(command: &mut Command, names: &[&str]) {
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

/// How many times [`spawn_with_busy_retry`] re-attempts a busy executable, and
/// the first backoff. Seven doublings from 1 ms give a 127 ms worst case, which
/// is bounded well below the shortest timeout any caller uses and still two
/// orders of magnitude more than the window it waits for.
const BUSY_SPAWN_RETRIES: u32 = 7;
const BUSY_SPAWN_FIRST_DELAY: Duration = Duration::from_millis(1);

/// Spawn `command`, retrying the one transient failure a concurrent `fork` can
/// cause on Linux.
///
/// `execve` refuses to start a file that still has a writer, and it enforces
/// that *before* the kernel closes the `O_CLOEXEC` descriptors the child
/// inherited. So a `fork` in another thread duplicates a write descriptor that
/// this thread is about to close, and this thread's `spawn` of the file it just
/// wrote fails with `ETXTBSY` (`ErrorKind::ExecutableFileBusy`) even though
/// nobody is still writing it. That is exactly the shape of the test helpers
/// that write a script and immediately run it: they are correct, and the
/// kernel's check is simply racy.
///
/// The descriptor is released as soon as the other child reaches its own
/// `exec`, so a short bounded retry is enough. Only `ExecutableFileBusy` is
/// retried; everything else — including the `NotFound` that callers map to
/// `Unavailable` — is returned untouched on the first attempt, and each retry
/// re-uses the identical command, so arguments, environment and stdio cannot
/// drift between attempts.
pub async fn spawn_with_busy_retry(command: &mut Command) -> std::io::Result<Child> {
    let mut delay = BUSY_SPAWN_FIRST_DELAY;
    let mut retries = 0;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && retries < BUSY_SPAWN_RETRIES =>
            {
                retries += 1;
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// Unique to this module: a concurrently running test cannot observe or
    /// clear it, so `set_var`/`remove_var` here cannot disturb anything else.
    const CANARY: &str = "AMATL_CHILD_PROCESS_CANARY_8f21";

    /// A second, distinct name for the forwarding test: sharing one variable
    /// between two tests would make the assertions race with each other's
    /// `remove_var`, which is exactly the flakiness this file must not have.
    const FORWARDED: &str = "AMATL_CHILD_PROCESS_FORWARD_2b93";

    /// Run `/bin/sh -c env` with whatever `build` applies and return the
    /// child's environment as text. `/bin/sh` is absolute so the spawn itself
    /// never depends on `PATH`; the `env` utility inside the script does.
    async fn child_environment(build: impl FnOnce(&mut Command)) -> String {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("env")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        build(&mut command);
        let output = command.output().await.expect("spawn /bin/sh");
        assert!(output.status.success(), "child failed: {output:?}");
        String::from_utf8(output.stdout).expect("env output is utf-8")
    }

    /// The credential a hostile document must never be able to read is exactly
    /// what the parent keeps in its environment.
    #[tokio::test]
    async fn scrubbing_removes_inherited_credentials_and_keeps_path() {
        std::env::set_var(CANARY, "server-token-canary");
        let environment = child_environment(scrub_environment).await;
        std::env::remove_var(CANARY);

        assert!(
            !environment.contains(CANARY),
            "the child inherited a credential: {environment}"
        );
        assert!(
            environment.lines().any(|line| line.starts_with("PATH=")),
            "PATH must survive the scrub or bare program names stop resolving: {environment}"
        );
    }

    /// Forwarding is how a spawn site opts a variable back in, and it only
    /// ever opts in the names it lists.
    #[tokio::test]
    async fn forwarding_readmits_only_the_named_variables() {
        std::env::set_var(FORWARDED, "forwarded-value");
        let environment = child_environment(|command| {
            scrub_environment(command);
            forward_environment(command, &[FORWARDED]);
        })
        .await;
        std::env::remove_var(FORWARDED);

        assert!(environment.contains(&format!("{FORWARDED}=forwarded-value")));
        assert!(!environment.contains(CANARY));
        assert!(!environment.contains("AMATL_SERVER_TOKEN"));
    }

    /// The allowlist is the security boundary, so it is asserted directly: a
    /// name that can hold a secret must not appear in it.
    #[test]
    fn the_allowlist_never_names_a_secret() {
        for name in INHERITED_ENV_ALLOWLIST {
            let upper = name.to_ascii_uppercase();
            assert!(
                !upper.contains("TOKEN")
                    && !upper.contains("SECRET")
                    && !upper.contains("KEY")
                    && !upper.contains("PASSWORD")
                    && !upper.contains("CREDENTIAL"),
                "{name} can hold a secret and must not be inherited"
            );
        }
    }
}
