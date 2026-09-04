// SPDX-License-Identifier: Apache-2.0

//! The external formatter protocol: one selected file's exact bytes go to
//! a repository-declared program on standard input, its standard output is
//! the canonical form, and a difference is a diagnostic. The program is a
//! trusted host process chosen by the repository; Bearout confines what it
//! sees, not what it can do.
//!
//! What Bearout controls:
//!
//! - the program runs from an argument vector, never a shell, with
//!   `{path}` in an argument replaced by the project-relative path so the
//!   tool can select language and configuration;
//! - its working directory is a private temporary directory holding only
//!   the `support` files the bootstrap names, read from the selected tree,
//!   so a staged or committed configuration governs an index or revision
//!   check even when the checkout differs; the directory is removed
//!   afterwards, and every temporary and cache location it is told about
//!   lies outside the target repository;
//! - it runs non-interactively with color disabled, sequentially in path
//!   order, with bounded standard input, output, and error, and a
//!   wall-clock bound after which it is killed and reaped;
//! - a missing executable is fatal; a non-zero exit, a timeout, oversized
//!   output, or abnormal termination is one diagnostic on the file.
//!
//! What Bearout does not control: the program's own reads and writes
//! elsewhere on the host. An authorized formatter is not confined by
//! Starlark's capability model, and Bearout is not a sandbox.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::bootstrap::{Formatter, PATH_PLACEHOLDER};
use crate::paths::ProjectPath;
use crate::tree::ReadTree;

/// Standard error retained from a formatter, in bytes.
const MAX_STDERR_BYTES: usize = 64 * 1024;
/// Standard output allowed beyond four times the input.
const OUTPUT_HEADROOM: usize = 1024 * 1024;
/// How often a running formatter is polled for exit.
const POLL: Duration = Duration::from_millis(5);

/// Why a formatter produced no usable output.
#[derive(Debug)]
pub enum Failure {
    /// The program could not be started; nothing can be checked.
    Start(String),
    /// The program exited with a non-zero status.
    Status(i32, String),
    /// The program was killed after the timeout.
    Timeout(Duration),
    /// The program wrote more than the bound allows.
    Oversized(usize),
    /// The program ended without an exit status.
    Abnormal(String),
    /// A pipe to or from the program failed.
    Io(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(detail) => write!(f, "cannot start: {detail}"),
            Self::Status(code, detail) if detail.is_empty() => {
                write!(f, "exited with status {code}")
            }
            Self::Status(code, detail) => write!(f, "exited with status {code}: {detail}"),
            Self::Timeout(limit) => write!(f, "timed out after {} s", limit.as_secs()),
            Self::Oversized(bound) => {
                write!(f, "produced more than {bound} bytes of standard output")
            }
            Self::Abnormal(detail) if detail.is_empty() => {
                f.write_str("ended without an exit status")
            }
            Self::Abnormal(detail) => write!(f, "ended without an exit status: {detail}"),
            Self::Io(detail) => write!(f, "pipe failed: {detail}"),
        }
    }
}

/// A private working directory for one formatter, populated with its
/// support files from the selected tree and removed on drop.
pub struct Workdir {
    root: PathBuf,
}

impl Workdir {
    /// Create the directory and copy every support file into it at its
    /// project-relative path. A support file that is missing, linked, or
    /// unreadable in the selected tree is an error naming it.
    pub fn prepare(tree: &dyn ReadTree, formatter: &Formatter) -> Result<Self, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let base = std::env::temp_dir();
        let mut created = None;
        for attempt in 0..16u32 {
            let root = base.join(format!(
                "bearout-format-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            match std::fs::create_dir(&root) {
                Ok(()) => {
                    created = Some(root);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "cannot create a working directory for formatter `{}`: {error}",
                        formatter.name
                    ));
                }
            }
        }
        let Some(root) = created else {
            return Err(format!(
                "cannot create a working directory for formatter `{}`: no free temporary name",
                formatter.name
            ));
        };
        let workdir = Self { root };
        for name in ["tmp", "cache"] {
            std::fs::create_dir(workdir.root.join(name)).map_err(|error| {
                format!(
                    "cannot prepare the working directory of formatter `{}`: {error}",
                    formatter.name
                )
            })?;
        }
        for support in &formatter.support {
            match tree.symlink_component(support) {
                Ok(None) => {}
                Ok(Some(link)) => {
                    return Err(format!(
                        "support file `{support}` of formatter `{}` is reached through the symbolic link `{link}`",
                        formatter.name
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "cannot inspect support file `{support}` of formatter `{}`: {error}",
                        formatter.name
                    ));
                }
            }
            let bytes = tree.read(support).map_err(|error| {
                format!(
                    "cannot read support file `{support}` of formatter `{}` from the selected tree: {error}",
                    formatter.name
                )
            })?;
            let target = workdir.root.join("files").join(support.to_native());
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot place support file `{support}`: {error}"))?;
            }
            std::fs::write(&target, bytes)
                .map_err(|error| format!("cannot place support file `{support}`: {error}"))?;
        }
        Ok(workdir)
    }

    /// Where the formatter runs: the support files live here at their
    /// project-relative paths.
    fn cwd(&self) -> PathBuf {
        self.root.join("files")
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Run `formatter` over `bytes` for the file at `path` and return the
/// canonical bytes.
pub fn run(
    formatter: &Formatter,
    workdir: &Workdir,
    path: &ProjectPath,
    bytes: &[u8],
) -> Result<Vec<u8>, Failure> {
    let cwd = workdir.cwd();
    let _ = std::fs::create_dir_all(&cwd);
    let temp = workdir.root.join("tmp");
    let cache = workdir.root.join("cache");
    let mut command = Command::new(&formatter.command[0]);
    for argument in &formatter.command[1..] {
        command.arg(argument.replace(PATH_PLACEHOLDER, path.as_str()));
    }
    command
        .current_dir(&cwd)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .env("TMPDIR", &temp)
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .env("XDG_CACHE_HOME", &cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| Failure::Start(describe_spawn(&formatter.command[0], &error)))?;

    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        // A formatter that exits early closes the pipe; that is its
        // status's story, not a write error.
        let result = stdin.write_all(&input);
        drop(stdin);
        result.or_else(|error| {
            if error.kind() == io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(error)
            }
        })
    });
    let bound = bytes
        .len()
        .saturating_mul(4)
        .saturating_add(OUTPUT_HEADROOM);
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().expect("stdout is piped");
    let stdout_overflow = Arc::clone(&overflow);
    let stdout_reader = std::thread::spawn(move || {
        let result = read_bounded(stdout, bound);
        if matches!(result, Ok((_, true))) {
            stdout_overflow.store(true, Ordering::Relaxed);
        }
        result
    });
    let stderr = child.stderr.take().expect("stderr is piped");
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr, MAX_STDERR_BYTES));

    let deadline = Instant::now() + formatter.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(Failure::Io(error.to_string())),
        }
        if overflow.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            break Err(Failure::Oversized(bound));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err(Failure::Timeout(formatter.timeout));
        }
        std::thread::sleep(POLL);
    };
    let written = writer.join().unwrap_or(Ok(()));
    let output = stdout_reader
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("output reader failed")));
    let errors = stderr_reader
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("error reader failed")))
        .map_or_else(|_| Vec::new(), |(bytes, _)| bytes);
    let status = status?;
    let (output, truncated) = output.map_err(|error| Failure::Io(error.to_string()))?;
    if truncated {
        return Err(Failure::Oversized(bound));
    }
    written.map_err(|error| Failure::Io(error.to_string()))?;
    match status.code() {
        Some(0) => Ok(output),
        Some(code) => Err(Failure::Status(code, crate::git::sanitize(&errors))),
        None => Err(Failure::Abnormal(crate::git::sanitize(&errors))),
    }
}

fn describe_spawn(program: &str, error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::NotFound {
        format!("`{program}` is not installed or not on PATH")
    } else {
        format!("`{program}`: {error}")
    }
}

/// Read up to `limit` bytes; the flag reports whether more were available.
/// On overflow the reader stops, which lets the caller kill the writer.
fn read_bounded(reader: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut buffer = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        buffer.truncate(limit);
        Ok((buffer, true))
    } else {
        Ok((buffer, false))
    }
}
