use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(super) struct SpoolBudget {
    root: PathBuf,
    process_limit: u64,
    total_limit: u64,
    used: Arc<Mutex<u64>>,
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_stored: u64,
    pub stderr_stored: u64,
    pub truncated: bool,
    pub capture_incomplete: bool,
    pub duration_ms: Option<u64>,
}
pub(super) struct Capture {
    directory: PathBuf,
    stdout: File,
    stderr: File,
    budget: SpoolBudget,
    metadata: Metadata,
}
pub(super) struct Saved {
    pub output: Vec<u8>,
    pub metadata: Metadata,
}
fn private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn create_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
impl SpoolBudget {
    pub fn new(root: &Path, process_limit: u64, total_limit: u64) -> io::Result<Self> {
        private_dir(root)?;
        let mut used = 0u64;
        let mut records = 0usize;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            records += 1;
            for stream in ["stdout", "stderr"] {
                if let Ok(metadata) = fs::metadata(entry.path().join(stream)) {
                    used = used.saturating_add(metadata.len());
                }
            }
        }
        if records > 4096 {
            return Err(io::Error::other(
                "spool record limit exceeded; explicit retention required",
            ));
        }
        Ok(Self {
            root: root.to_owned(),
            process_limit,
            total_limit,
            used: Arc::new(Mutex::new(used)),
        })
    }
    pub fn create(&self, id: &str) -> io::Result<Capture> {
        // Bound metadata/inodes even once the payload quota has been exhausted.
        if fs::read_dir(&self.root)?.count() >= 4096 {
            return Err(io::Error::other(
                "spool record limit reached; explicit retention required",
            ));
        }
        let directory = self.root.join(id);
        fs::create_dir(&directory)?;
        private_dir(&directory)?;
        let capture = Capture {
            stdout: create_file(&directory.join("stdout"))?,
            stderr: create_file(&directory.join("stderr"))?,
            directory,
            budget: self.clone(),
            metadata: Metadata::default(),
        };
        capture.save_metadata()?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(capture)
    }
    pub fn read(&self, id: &str, stderr: bool, capacity: usize) -> io::Result<Saved> {
        let directory = self.root.join(id);
        let mut metadata: Metadata =
            serde_json::from_slice(&fs::read(directory.join("metadata.json"))?)?;
        // A crash between synced output and metadata commit preserves output; mark the accounting gap.
        for (name, stored, seen) in [
            (
                "stdout",
                &mut metadata.stdout_stored,
                &mut metadata.stdout_bytes,
            ),
            (
                "stderr",
                &mut metadata.stderr_stored,
                &mut metadata.stderr_bytes,
            ),
        ] {
            let actual = fs::metadata(directory.join(name))?.len();
            if actual != *stored {
                metadata.capture_incomplete = true;
                *stored = actual;
                *seen = (*seen).max(actual);
            }
        }
        let mut file = File::open(directory.join(if stderr { "stderr" } else { "stdout" }))?;
        let len = file.metadata()?.len();
        let start = len.saturating_sub(capacity as u64);
        file.seek(SeekFrom::Start(start))?;
        let mut output = Vec::new();
        file.take(capacity as u64).read_to_end(&mut output)?;
        Ok(Saved { output, metadata })
    }
}
impl Capture {
    pub fn finish(&mut self, duration_ms: u64, incomplete: bool) -> io::Result<()> {
        self.metadata.duration_ms = Some(duration_ms);
        self.metadata.capture_incomplete |= incomplete;
        self.save_metadata()
    }
    fn save_metadata(&self) -> io::Result<()> {
        let temporary = self.directory.join("metadata.tmp");
        // Only this capture owns the directory. A leftover tmp after crash is ignored.
        if temporary.exists() {
            fs::remove_file(&temporary)?;
        }
        let mut file = create_file(&temporary)?;
        file.write_all(&serde_json::to_vec(&self.metadata)?)?;
        file.sync_all()?;
        fs::rename(temporary, self.directory.join("metadata.json"))?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
    pub fn append(&mut self, stderr: bool, bytes: &[u8]) -> io::Result<()> {
        let mut used = self
            .budget
            .used
            .lock()
            .map_err(|_| io::Error::other("spool budget poisoned"))?;
        let stored = self.metadata.stdout_stored + self.metadata.stderr_stored;
        let allowed = (bytes.len() as u64)
            .min(self.budget.process_limit.saturating_sub(stored))
            .min(self.budget.total_limit.saturating_sub(*used)) as usize;
        let file = if stderr {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        // Reserve before writing: a partial disk write must never permit a later
        // capture to exceed the shared budget.
        *used += allowed as u64;
        file.write_all(&bytes[..allowed])?;
        file.flush()?;
        file.sync_data()?;
        drop(used);
        if stderr {
            self.metadata.stderr_bytes += bytes.len() as u64;
            self.metadata.stderr_stored += allowed as u64;
        } else {
            self.metadata.stdout_bytes += bytes.len() as u64;
            self.metadata.stdout_stored += allowed as u64;
        }
        self.metadata.truncated |= allowed < bytes.len();
        self.save_metadata()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_is_durable_before_process_exit_and_quota_is_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let budget = SpoolBudget::new(dir.path(), 5, 8).unwrap();
        let mut first = budget.create("first").unwrap();
        first.append(false, b"abcdef").unwrap();
        let saved = budget.read("first", false, 100).unwrap();
        assert_eq!(saved.output, b"abcde");
        assert_eq!(saved.metadata.stdout_bytes, 6);
        assert!(saved.metadata.truncated);
        let mut second = budget.create("second").unwrap();
        second.append(true, b"12345").unwrap();
        assert_eq!(budget.read("second", true, 100).unwrap().output, b"123");
        drop(first);
        drop(second);
        let reopened = SpoolBudget::new(dir.path(), 5, 8).unwrap();
        assert_eq!(reopened.read("first", false, 2).unwrap().output, b"de");
        let mut third = reopened.create("third").unwrap();
        third.append(false, b"x").unwrap();
        assert!(
            reopened
                .read("third", false, 100)
                .unwrap()
                .output
                .is_empty()
        );
    }
    #[test]
    fn crash_between_data_sync_and_metadata_commit_preserves_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let budget = SpoolBudget::new(dir.path(), 100, 100).unwrap();
        let capture = budget.create("crash").unwrap();
        drop(capture);
        let mut output = OpenOptions::new()
            .append(true)
            .open(dir.path().join("crash/stdout"))
            .unwrap();
        output.write_all(b"confirmed-output").unwrap();
        output.sync_data().unwrap();
        drop(output);
        let reopened = SpoolBudget::new(dir.path(), 100, 100).unwrap();
        let saved = reopened.read("crash", false, 100).unwrap();
        assert_eq!(saved.output, b"confirmed-output");
        assert!(saved.metadata.capture_incomplete);
        assert_eq!(saved.metadata.stdout_stored, 16);
    }
}
