use std::fs::File;
use std::io::{stdout, StdoutLock, Write};
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, FromRawFd};

pub struct UnbufferedStdout {
    // false positive, we don't use the value, but want to hold the lock
    #[allow(dead_code)]
    lock: StdoutLock<'static>,
    out: ManuallyDrop<File>,
}

impl UnbufferedStdout {
    pub fn lock() -> Self {
        let lock = stdout().lock();
        // SAFETY: The standard library relies on the assumption the stdout fd is valid, so we have
        // to rely on this too. Closing the stdout fd, would break the standard library, so we make
        // sure to not drop it by wrapping it in ManuallyDrop.
        let out = ManuallyDrop::new(unsafe { File::from_raw_fd(lock.as_raw_fd()) });
        Self { lock, out }
    }

    pub fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.out.write_all(buf)
    }
}
