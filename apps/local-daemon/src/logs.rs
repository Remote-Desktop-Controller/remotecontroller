use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
pub struct Writer(Arc<Mutex<(PathBuf, File)>>);
impl Writer {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self(Arc::new(Mutex::new((path, file)))))
    }
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut guard = self.0.lock().map_err(|_| io::Error::other("log lock"))?;
        if guard.1.metadata()?.len() + bytes.len() as u64 > 1024 * 1024 {
            // Two bounded files; rename is unnecessary and works with open Windows handles.
            let old = std::fs::read(&guard.0)?;
            std::fs::write(guard.0.with_extension("log.1"), old)?;
            guard.1.set_len(0)?;
        }
        guard.1.write(&bytes[..bytes.len().min(1024 * 1024)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log lock"))?
            .1
            .flush()
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Writer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}
