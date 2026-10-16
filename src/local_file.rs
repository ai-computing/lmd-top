//! Local manifest files: exclusive saves and isolated, private editor sessions.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn write_new(path: &Path, content: &str) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

pub fn save_manifest(directory: &Path, title: &str, content: &str) -> io::Result<PathBuf> {
    let stem: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    let stem = if stem.is_empty() {
        "manifest"
    } else {
        &stem[..stem.len().min(48)]
    };
    for attempt in 0..1000 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let path = directory.join(format!("lmd-{stem}{suffix}.yaml"));
        match write_new(&path, content) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many saved manifests with this title",
    ))
}

pub struct EditFile {
    directory: PathBuf,
    path: PathBuf,
}
impl EditFile {
    pub fn new(content: &str) -> io::Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..100 {
            let directory = std::env::temp_dir().join(format!(
                "lmd-edit-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    let file = Self {
                        path: directory.join("manifest.yaml"),
                        directory,
                    };
                    write_new(&file.path, content)?;
                    return Ok(file);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot allocate private editor directory",
        ))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for EditFile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeat_saves_preserve_existing_content_and_editor_sessions_are_isolated() {
        let first = EditFile::new("first").unwrap();
        let second = EditFile::new("second").unwrap();
        assert_ne!(first.path(), second.path());
        let a = save_manifest(&first.directory, "../같은 title", "saved-a").unwrap();
        let b = save_manifest(&first.directory, "../같은 title", "saved-b").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(first.directory.as_path()));
        assert_eq!(fs::read_to_string(a).unwrap(), "saved-a");
        assert_eq!(fs::read_to_string(b).unwrap(), "saved-b");
        let path = first.path().to_owned();
        drop(first);
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(second.path()).unwrap(), "second");
    }
    #[cfg(unix)]
    #[test]
    fn saves_do_not_follow_existing_symlinks_and_files_are_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let session = EditFile::new("victim").unwrap();
        symlink(session.path(), session.directory.join("lmd-test.yaml")).unwrap();
        let saved = save_manifest(&session.directory, "test", "new").unwrap();
        assert_ne!(saved, session.directory.join("lmd-test.yaml"));
        assert_eq!(fs::read_to_string(session.path()).unwrap(), "victim");
        assert_eq!(
            fs::metadata(saved).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(session.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&session.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
