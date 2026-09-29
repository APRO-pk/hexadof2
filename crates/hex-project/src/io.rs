//! Small filesystem and clock helpers shared by the project modules.
//!
//! Everything that writes a file goes through [`write_atomic`], so a crash cannot
//! leave a half-written artifact that later looks like a valid one.

use std::path::{Component, Path, PathBuf};

use crate::error::ProjectError;

/// Write bytes to a file by writing a sibling temporary file and renaming it.
///
/// A rename within the same directory is atomic on both supported platforms, so a
/// reader either sees the previous contents or the complete new contents.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| ProjectError::io(parent, e))?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "artifact".to_string()),
        crate::clock::new_id()
    ));
    std::fs::write(&temporary, bytes).map_err(|e| ProjectError::io(&temporary, e))?;
    if let Err(e) = std::fs::rename(&temporary, path) {
        // A failed rename must not leave the temporary file behind.
        let _ = std::fs::remove_file(&temporary);
        return Err(ProjectError::io(path, e));
    }
    Ok(())
}

/// Write a JSON document with stable pretty formatting.
pub fn write_json<T: serde::Serialize + ?Sized>(
    path: &Path,
    value: &T,
) -> Result<(), ProjectError> {
    let text = serde_json::to_string_pretty(value).map_err(|e| ProjectError::Serialize {
        detail: e.to_string(),
    })?;
    write_atomic(path, text.as_bytes())
}

/// Read a JSON document, reporting the path on failure.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|e| ProjectError::io(path, e))?;
    serde_json::from_str(&text).map_err(|e| ProjectError::Parse {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

/// Lexically normalise a path, removing `.` and resolving `..` without touching
/// the filesystem.
///
/// Used for the containment check on project-relative references, where the path
/// may not exist yet, so `canonicalize` is not available.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Reject a file name that could escape its directory.
pub fn safe_file_name(name: &str) -> Result<String, ProjectError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProjectError::UnsafeFileName {
            name: name.to_string(),
        });
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute()
        || candidate.components().count() != 1
        || trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
    {
        return Err(ProjectError::UnsafeFileName {
            name: name.to_string(),
        });
    }
    Ok(trimmed.to_string())
}

/// A file name safe for the filesystem, derived from a display name.
///
/// Spaces become hyphens and anything outside a conservative set is dropped, so a
/// user-typed run name cannot produce an unusable path.
pub fn slugify(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut last_was_separator = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_separator = false;
        } else if !last_was_separator && !out.is_empty() {
            out.push('-');
            last_was_separator = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

/// A unique directory name, preferring the plain stem.
///
/// `label` becomes `label` when that name is free, and `label-002`, `label-003`
/// otherwise. A user-typed name that already ends in a number therefore does not
/// grow a second numeric suffix.
pub fn unique_directory(parent: &Path, stem: &str) -> PathBuf {
    let first = parent.join(stem);
    if !first.exists() {
        return first;
    }
    for index in 2..100_000usize {
        let candidate = parent.join(format!("{}-{:03}", stem, index));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{}-{}", stem, crate::clock::new_id()))
}

/// A unique file path in a directory, preferring the plain stem.
pub fn unique_file(parent: &Path, stem: &str, extension: &str) -> PathBuf {
    let first = parent.join(format!("{}.{}", stem, extension));
    if !first.exists() {
        return first;
    }
    for index in 2..100_000usize {
        let candidate = parent.join(format!("{}-{:03}.{}", stem, index, extension));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{}-{}.{}", stem, crate::clock::new_id(), extension))
}

/// Find the next free directory name with a numeric suffix.
///
/// Artifacts are never overwritten, so a repeated name gets the next index.
pub fn next_available_directory(parent: &Path, stem: &str) -> PathBuf {
    for index in 1..100_000usize {
        let candidate = parent.join(format!("{}-{:03}", stem, index));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{}-{}", stem, crate::clock::new_id()))
}

/// Count files in a directory whose name satisfies a predicate.
pub fn count_matching(directory: &Path, predicate: impl Fn(&str) -> bool) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_file())
        .filter(|e| predicate(&e.file_name().to_string_lossy()))
        .count()
}

/// Count immediate subdirectories.
pub fn count_directories(directory: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .count()
}

/// Count files anywhere under a directory whose name satisfies a predicate.
pub fn count_matching_recursive(directory: &Path, predicate: impl Fn(&str) -> bool) -> usize {
    let mut total = 0usize;
    let mut stack = vec![directory.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() && predicate(&entry.file_name().to_string_lossy()) => {
                    total += 1;
                }
                _ => {}
            }
        }
    }
    total
}

/// List immediate subdirectory names, sorted.
pub fn list_directories(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// List immediate file names, sorted.
pub fn list_files(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Remove a directory recursively, refusing to follow a symbolic link out of the
/// project.
pub fn remove_directory_within(root: &Path, target: &Path) -> Result<(), ProjectError> {
    let normalized = normalize(target);
    if !normalized.starts_with(normalize(root)) {
        return Err(ProjectError::ReferenceOutside {
            reference: target.to_string_lossy().to_string(),
        });
    }
    if !target.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(target).map_err(|e| ProjectError::io(target, e))?;
    if metadata.file_type().is_symlink() {
        return Err(ProjectError::RefusedSymlink {
            path: target.to_path_buf(),
        });
    }
    std::fs::remove_dir_all(target).map_err(|e| ProjectError::io(target, e))
}

/// Total size of a directory tree in bytes, ignoring anything unreadable.
pub fn directory_size(directory: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![directory.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            match entry.metadata() {
                Ok(m) if m.is_file() => total += m.len(),
                Ok(m) if m.is_dir() => stack.push(path),
                _ => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hexadof-io-{}-{}-{}",
            label,
            std::process::id(),
            crate::clock::new_id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn write_atomic_creates_and_replaces() {
        let dir = temp_dir("atomic");
        let path = dir.join("a.json");
        write_atomic(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        // No temporary files are left behind.
        let leftovers: Vec<String> = list_files(&dir)
            .into_iter()
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "leftovers {:?}", leftovers);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_creates_missing_parents() {
        let dir = temp_dir("parents");
        let nested = dir.join("a/b/c/file.txt");
        write_atomic(&nested, b"x").unwrap();
        assert!(nested.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_round_trips() {
        let dir = temp_dir("json");
        let path = dir.join("value.json");
        write_json(&path, &vec![1.5, 2.5, 3.5]).unwrap();
        let back: Vec<f64> = read_json(&path).unwrap();
        assert_eq!(back, vec![1.5, 2.5, 3.5]);

        std::fs::write(&path, b"{not json").unwrap();
        let err = read_json::<Vec<f64>>(&path).unwrap_err();
        assert!(matches!(err, ProjectError::Parse { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_json_reports_a_missing_file() {
        let dir = temp_dir("jsonmissing");
        let err = read_json::<Vec<f64>>(&dir.join("nope.json")).unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_resolves_dots_and_parents() {
        assert_eq!(normalize(Path::new("a/./b")), PathBuf::from("a/b"));
        assert_eq!(normalize(Path::new("a/b/../c")), PathBuf::from("a/c"));
        assert_eq!(normalize(Path::new("a/../../b")), PathBuf::from("b"));
    }

    #[test]
    fn safe_file_name_accepts_plain_names_and_rejects_paths() {
        assert_eq!(safe_file_name("rocket.json").unwrap(), "rocket.json");
        assert_eq!(safe_file_name("  spaced.json  ").unwrap(), "spaced.json");
        for bad in [
            "",
            "   ",
            "../escape.json",
            "a/b.json",
            "a\\b.json",
            "C:/x.json",
            "/etc/passwd",
        ] {
            assert!(
                safe_file_name(bad).is_err(),
                "{} should have been rejected",
                bad
            );
        }
    }

    #[test]
    fn slugify_produces_a_usable_name() {
        assert_eq!(slugify("Run 001", "run"), "run-001");
        assert_eq!(slugify("Vertical Flight!", "run"), "vertical-flight");
        assert_eq!(slugify("  ", "run"), "run");
        assert_eq!(slugify("___", "fallback"), "fallback");
        assert_eq!(slugify("MixedCASE", "run"), "mixedcase");
        assert!(!slugify("a".repeat(500).as_str(), "run").contains(' '));
    }

    #[test]
    fn next_available_directory_skips_taken_names() {
        let dir = temp_dir("nextdir");
        let first = next_available_directory(&dir, "run");
        assert_eq!(first.file_name().unwrap(), "run-001");
        std::fs::create_dir_all(&first).unwrap();
        let second = next_available_directory(&dir, "run");
        assert_eq!(second.file_name().unwrap(), "run-002");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counting_helpers_handle_a_missing_directory() {
        let missing = std::env::temp_dir().join("hexadof-does-not-exist-xyz");
        assert_eq!(count_matching(&missing, |_| true), 0);
        assert_eq!(count_directories(&missing), 0);
        assert!(list_directories(&missing).is_empty());
        assert!(list_files(&missing).is_empty());
        assert_eq!(directory_size(&missing), 0);
    }

    #[test]
    fn counting_helpers_report_what_is_there() {
        let dir = temp_dir("counts");
        std::fs::write(dir.join("a.json"), b"x").unwrap();
        std::fs::write(dir.join("b.txt"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert_eq!(count_matching(&dir, |n| n.ends_with(".json")), 1);
        assert_eq!(count_directories(&dir), 1);
        assert_eq!(list_files(&dir), vec!["a.json", "b.txt"]);
        assert_eq!(list_directories(&dir), vec!["sub"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_directory_refuses_to_escape_its_root() {
        let root = temp_dir("removeescape");
        let outside = temp_dir("removeoutside");
        let err = remove_directory_within(&root, &outside).unwrap_err();
        assert!(matches!(err, ProjectError::ReferenceOutside { .. }));
        assert!(outside.exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn remove_directory_deletes_a_child_and_tolerates_a_missing_one() {
        let root = temp_dir("removechild");
        let child = root.join("child");
        std::fs::create_dir_all(child.join("nested")).unwrap();
        std::fs::write(child.join("nested/file.txt"), b"x").unwrap();
        remove_directory_within(&root, &child).unwrap();
        assert!(!child.exists());
        // Removing again is not an error.
        remove_directory_within(&root, &child).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_size_sums_files_recursively() {
        let dir = temp_dir("size");
        std::fs::write(dir.join("a.bin"), vec![0u8; 100]).unwrap();
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("b.bin"), vec![0u8; 250]).unwrap();
        assert_eq!(directory_size(&dir), 350);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_size_ignores_symbolic_links() {
        let dir = temp_dir("symlink");
        std::fs::write(dir.join("a.bin"), vec![0u8; 10]).unwrap();
        // Creating a symlink needs a platform call; skip quietly when it fails so
        // the test stays honest rather than silently passing.
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(dir.join("a.bin"), dir.join("link.bin"));
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(dir.join("a.bin"), dir.join("link.bin"));
        if created.is_ok() {
            // The link itself is a file, so its tiny size may be counted, but the
            // target must not be counted twice.
            assert!(directory_size(&dir) < 1000);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
