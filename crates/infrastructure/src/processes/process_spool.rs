use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct SpoolBudget {
    root: PathBuf,
    process_limit: u64,
    total_limit: u64,
    used: Arc<Mutex<u64>>,
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Metadata {
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_stored: u64,
    pub stderr_stored: u64,
    pub truncated: bool,
    pub capture_incomplete: bool,
    pub duration_ms: Option<u64>,
}
pub(crate) struct Capture {
    directory: PathBuf,
    stdout: File,
    stderr: File,
    budget: SpoolBudget,
    metadata: Metadata,
}
pub(crate) struct Saved {
    pub output: Vec<u8>,
    pub metadata: Metadata,
}
pub(crate) struct Range {
    pub output: Vec<u8>,
    pub metadata: Metadata,
    pub offset: u64,
    pub next_offset: Option<u64>,
}
pub(crate) struct SearchOptions {
    pub offset: u64,
    pub scan_bytes: usize,
    pub context_bytes: usize,
    pub limit: usize,
    pub budget_bytes: usize,
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
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
        let file = self
            .root
            .join(id)
            .join(if stderr { "stderr" } else { "stdout" });
        let offset = fs::metadata(file)?.len().saturating_sub(capacity as u64);
        let range = self.read_range(id, stderr, offset, capacity)?;
        Ok(Saved {
            output: range.output,
            metadata: range.metadata,
        })
    }
    pub fn read_range(
        &self,
        id: &str,
        stderr: bool,
        offset: u64,
        capacity: usize,
    ) -> io::Result<Range> {
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
        file.seek(SeekFrom::Start(offset))?;
        let mut output = Vec::new();
        file.take(capacity as u64).read_to_end(&mut output)?;
        let end = offset.saturating_add(output.len() as u64);
        Ok(Range {
            output,
            metadata,
            offset,
            next_offset: if end < len { Some(end) } else { None },
        })
    }
    pub fn search(
        &self,
        id: &str,
        stderr: bool,
        query: &[u8],
        options: SearchOptions,
    ) -> io::Result<serde_json::Value> {
        let start = options.offset.saturating_sub(options.context_bytes as u64);
        let capacity = (options.offset - start) as usize
            + options.scan_bytes
            + query.len().saturating_sub(1)
            + options.context_bytes;
        let range = self.read_range(id, stderr, start, capacity)?;
        let stored = if stderr {
            range.metadata.stderr_stored
        } else {
            range.metadata.stdout_stored
        };
        let generated = if stderr {
            range.metadata.stderr_bytes
        } else {
            range.metadata.stdout_bytes
        };
        let scan_end = options
            .offset
            .saturating_add(options.scan_bytes as u64)
            .min(stored);
        let mut matches = Vec::new();
        let mut used = 0;
        let mut stopped = None;
        if !query.is_empty() && range.output.len() >= query.len() {
            for index in (options.offset - start) as usize
                ..range.output.len().saturating_sub(query.len()) + 1
            {
                let offset = start + index as u64;
                if offset >= scan_end {
                    break;
                }
                if range.output[index..index + query.len()] != *query {
                    continue;
                }
                if matches.len() == options.limit {
                    stopped = Some(offset);
                    break;
                }
                let left = index.saturating_sub(options.context_bytes);
                let right = (index + query.len() + options.context_bytes).min(range.output.len());
                let entry = serde_json::json!({"offset":offset,"end_offset":offset+query.len() as u64,"context_offset":start+left as u64,"context_end_offset":start+right as u64,"encoding":"hex","data":hex(&range.output[left..right]),"text_encoding":"utf-8-lossy","text":String::from_utf8_lossy(&range.output[left..right])});
                let bytes = serde_json::to_vec(&entry)?.len();
                if used + bytes > options.budget_bytes {
                    stopped = Some(offset);
                    break;
                }
                used += bytes;
                matches.push(entry);
            }
        }
        let next = stopped.or(if scan_end < stored {
            Some(scan_end)
        } else {
            None
        });
        Ok(
            serde_json::json!({"matches":matches,"offset":options.offset,"scanned_until":stopped.unwrap_or(scan_end),"next_offset":next,"stored_bytes":stored,"generated_bytes":generated,"truncated":range.metadata.truncated || next.is_some(),"capture_incomplete":range.metadata.capture_incomplete,"bytes_used":used,"budget_bytes":options.budget_bytes,"duration_ms":range.metadata.duration_ms}),
        )
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
    fn range_is_byte_exact_and_search_matches_crossing_scan_boundary_are_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let spool = SpoolBudget::new(dir.path(), 1024, 2048).unwrap();
        let mut capture = spool.create("ranges").unwrap();
        capture
            .append(false, "áabcneedle-needle".as_bytes())
            .unwrap();
        capture.finish(1, false).unwrap();
        let split = spool.read_range("ranges", false, 1, 2).unwrap();
        assert_eq!(hex(&split.output), "a161");
        assert_eq!(split.next_offset, Some(3));
        let make_options = |offset, scan_bytes, budget_bytes| SearchOptions {
            offset,
            scan_bytes,
            context_bytes: 2,
            limit: 20,
            budget_bytes,
        };
        let page = spool
            .search("ranges", false, b"needle", make_options(0, 6, 32768))
            .unwrap();
        assert_eq!(page["matches"][0]["offset"], 5);
        assert_eq!(page["next_offset"], 6);
        let next = spool
            .search("ranges", false, b"needle", make_options(6, 1024, 32768))
            .unwrap();
        assert_eq!(next["matches"].as_array().unwrap().len(), 1);
        assert_eq!(next["matches"][0]["offset"], 12);
        let bounded = spool
            .search("ranges", false, b"needle", make_options(0, 1024, 0))
            .unwrap();
        assert!(bounded["matches"].as_array().unwrap().is_empty());
        assert_eq!(bounded["next_offset"], 5);
        assert_eq!(bounded["bytes_used"], 0);
        assert_eq!(bounded["truncated"], true);
    }

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
