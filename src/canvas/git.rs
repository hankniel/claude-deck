//! Git changes of a session's project: status with line counts, and per-file diffs.

use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

fn git(cwd: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct File {
    pub path: String,
    pub status: String,
    pub letter: String,
    pub staged: bool,
    pub untracked: bool,
    pub added: u64,
    pub deleted: u64,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Status {
    pub root: String,
    pub branch: String,
    pub files: Vec<File>,
    pub added: u64,
    pub deleted: u64,
}

pub fn root(cwd: &Path) -> Option<String> {
    let r = git(cwd, &["rev-parse", "--show-toplevel"]).trim().to_string();
    (!r.is_empty()).then_some(r)
}

pub fn status(cwd: &Path) -> Option<Status> {
    let root = root(cwd)?;
    let rp = Path::new(&root);
    let branch = git(rp, &["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_string();
    let mut counts: HashMap<String, (u64, u64)> = HashMap::new();
    for flag in [&[][..], &["--cached"][..]] {
        let mut args = vec!["diff"];
        args.extend_from_slice(flag);
        args.push("--numstat");
        for line in git(rp, &args).lines() {
            let p: Vec<&str> = line.split('\t').collect();
            if p.len() >= 3 {
                let e = counts.entry(p[2].to_string()).or_default();
                e.0 += p[0].parse::<u64>().unwrap_or(0);
                e.1 += p[1].parse::<u64>().unwrap_or(0);
            }
        }
    }
    let mut files = vec![];
    for line in git(rp, &["status", "--porcelain=v1", "--untracked-files=all"]).lines() {
        if line.len() < 4 {
            continue;
        }
        let x = &line[0..1];
        let y = &line[1..2];
        let path = line[3..].trim_matches('"').rsplit(" -> ").next().unwrap_or("").to_string();
        let letter = if x != " " && x != "?" { x } else if y != " " { y } else { x };
        let status = match letter {
            "M" => "modified",
            "A" => "added",
            "D" => "deleted",
            "R" => "renamed",
            "C" => "copied",
            "?" => "untracked",
            "U" => "conflict",
            _ => "changed",
        };
        let (mut added, deleted) = counts.get(&path).copied().unwrap_or((0, 0));
        if x == "?" {
            added = std::fs::read_to_string(rp.join(&path)).map(|t| t.lines().count() as u64).unwrap_or(0);
        }
        files.push(File {
            path,
            status: status.into(),
            letter: if letter == "?" { "U".into() } else { letter.into() },
            staged: x != " " && x != "?",
            untracked: x == "?",
            added,
            deleted,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let (added, deleted) = files.iter().fold((0, 0), |(a, d), f| (a + f.added, d + f.deleted));
    Some(Status { root, branch, files, added, deleted })
}

/// Unified diff of one file against HEAD (an untracked file against nothing).
pub fn diff(cwd: &Path, file: &str, untracked: bool) -> String {
    let Some(root) = root(cwd) else { return String::new() };
    let rp = Path::new(&root);
    if untracked {
        return git(rp, &["diff", "--no-index", "--", "/dev/null", file]);
    }
    let d = git(rp, &["diff", "HEAD", "--", file]);
    if d.is_empty() {
        git(rp, &["diff", "--", file])
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_in_a_temp_repo() {
        let dir = std::env::temp_dir().join(format!("deck-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |a: &[&str]| {
            Command::new("git").args(a).current_dir(&dir).output().unwrap();
        };
        run(&["init", "-q"]);
        run(&["-c", "user.email=a@b", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "x"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let s = status(&dir).unwrap();
        assert_eq!(s.files.len(), 1);
        assert!(s.files[0].untracked && s.files[0].added == 2);
        assert!(diff(&dir, "a.txt", true).contains("+one"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
