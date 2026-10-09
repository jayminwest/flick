//! Local-only git: what the checkout holds compared with the installed build. Every call
//! goes through `git()`, which kills the process at a deadline and runs only the
//! subcommands in `ALLOWED`. None of those touch the network; Flick sends nothing.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The only git subcommands Flick runs. All of them read the local repository.
pub const ALLOWED: [&str; 6] = ["rev-parse", "rev-list", "log", "status", "archive", "merge-base"];

/// How many newer commits' subjects a status keeps.
const SUBJECTS: &str = "5";

/// The checkout compared with the installed build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Full sha of the checkout's HEAD.
    pub head: String,
    /// The work tree has uncommitted changes.
    pub dirty: bool,
    /// Commits in `<installed>..HEAD`. `None` when there is no installed sha, or the
    /// checkout does not know it.
    pub newer: Option<usize>,
    /// The newest of those commits, newest first: (short sha, subject).
    pub subjects: Vec<(String, String)>,
}

/// Compare checkout `source` with the `installed` sha, within `budget` for all git calls.
pub fn check(source: &Path, installed: Option<&str>, budget: Duration) -> Result<Status, String> {
    let deadline = Instant::now() + budget;
    let run = |args: &[&str]| git(source, args, deadline);
    let head = parse_head(&run(&["rev-parse", "--verify", "HEAD"])?)?;
    let dirty = !run(&["status", "--porcelain"])?.trim().is_empty();
    let mut status = Status { head, dirty, ..Status::default() };
    match installed {
        Some(sha) if sha == status.head => status.newer = Some(0),
        Some(sha) => {
            let range = format!("{sha}..HEAD");
            // An installed sha this checkout does not have (another clone) is not an error.
            if let Ok(count) = run(&["rev-list", "--count", &range]) {
                status.newer = count.trim().parse().ok();
                status.subjects = parse_log(&run(&["log", "--format=%h%x09%s", "-n", SUBJECTS, &range])?);
            }
        }
        None => {}
    }
    Ok(status)
}

/// Run `git -C <source> <args>` and return its stdout. `args[0]` must be in `ALLOWED`.
/// Kills git at `deadline`. Optional locks are off, so a check never blocks an agent's
/// git command on the index lock.
pub fn git(source: &Path, args: &[&str], deadline: Instant) -> Result<String, String> {
    let sub = args.first().copied().unwrap_or_default();
    if !ALLOWED.contains(&sub) {
        return Err(format!("git {sub}: not allowed"));
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    let Some(mut stdout) = child.stdout.take() else { return Err("git: no stdout".into()) };
    // Read on a thread so a full pipe never stalls git while we wait for it to exit.
    let reader = thread::spawn(move || {
        let mut out = String::new();
        stdout.read_to_string(&mut out).map(|_| out)
    });
    loop {
        match child.try_wait() {
            Ok(Some(exit)) if exit.success() => {
                return reader
                    .join()
                    .map_err(|_| format!("git {sub}: reader panicked"))?
                    .map_err(|e| format!("git {sub}: {e}"));
            }
            Ok(Some(_)) => return Err(format!("git {sub} failed in {}", source.display())),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git {sub}: timed out"));
            }
            Err(e) => return Err(format!("git {sub}: {e}")),
        }
    }
}

/// The sha `git rev-parse HEAD` printed.
fn parse_head(out: &str) -> Result<String, String> {
    let head = out.trim();
    if head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(head.to_string())
    } else {
        Err(format!("git rev-parse: unexpected output {head:?}"))
    }
}

/// `git log --format=%h%x09%s` lines as (short sha, subject).
fn parse_log(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(sha, subject)| (sha.to_string(), subject.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }

    /// A throwaway repository with commits `one` and `two`; returns it and their shas.
    fn repo(name: &str) -> (PathBuf, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("flk-{}-git-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Setup needs init and commit, which `git` refuses; that is the point of `ALLOWED`.
        let sh = |args: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        sh(&["init", "-q"]);
        let mut shas = vec![];
        for subject in ["one", "two"] {
            sh(&["commit", "-q", "--allow-empty", "-m", subject]);
            shas.push(git(&dir, &["rev-parse", "HEAD"], soon()).unwrap().trim().to_string());
        }
        (dir, shas)
    }

    #[test]
    fn the_allow_list_is_local_only() {
        assert_eq!(ALLOWED, ["rev-parse", "rev-list", "log", "status", "archive", "merge-base"]);
        for sub in ["fetch", "pull", "push", "clone", "remote", "ls-remote", "submodule", "-c", ""] {
            let err = git(Path::new("/"), &[sub, "origin"], soon()).unwrap_err();
            assert!(err.ends_with("not allowed"), "{sub}: {err}");
        }
        assert!(git(Path::new("/"), &[], soon()).is_err());
    }

    #[test]
    fn check_counts_commits_newer_than_the_installed_sha() {
        let (dir, shas) = repo("check");
        let s = check(&dir, Some(&shas[0]), Duration::from_secs(10)).unwrap();
        assert_eq!(s.head, shas[1]);
        assert!(!s.dirty);
        assert_eq!(s.newer, Some(1));
        assert_eq!(s.subjects, [(shas[1][..s.subjects[0].0.len()].to_string(), "two".into())]);

        let same = check(&dir, Some(&shas[1]), Duration::from_secs(10)).unwrap();
        assert_eq!((same.newer, same.subjects.len()), (Some(0), 0));
        let unknown = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(check(&dir, Some(unknown), Duration::from_secs(10)).unwrap().newer, None);
        assert_eq!(check(&dir, None, Duration::from_secs(10)).unwrap().newer, None);

        std::fs::write(dir.join("edit.txt"), "x").unwrap();
        assert!(check(&dir, None, Duration::from_secs(10)).unwrap().dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_fails_outside_a_repository_and_past_the_deadline() {
        let dir = std::env::temp_dir().join(format!("flk-{}-git-none", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check(&dir, None, Duration::from_secs(10)).unwrap_err().contains("failed"));
        let late = git(&dir, &["status"], Instant::now()).unwrap_err();
        assert!(late.contains("timed out") || late.contains("failed"), "{late}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_head_and_log() {
        assert!(parse_head("abc\n").is_err());
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(parse_head(&format!("{sha}\n")).unwrap(), sha);
        let log = parse_log("abc1234\tfeat: a\tb\n\nnot a line\ndef5678\tfix\n");
        assert_eq!(log, [("abc1234".into(), "feat: a\tb".into()), ("def5678".into(), "fix".into())]);
    }
}
