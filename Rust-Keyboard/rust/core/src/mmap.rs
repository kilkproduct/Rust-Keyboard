//! Read-only file mapping.
//!
//! This is the only place in `kb-core` that talks to the operating system, and
//! it only ever asks for `PROT_READ` on a file the caller named.  Mapping a
//! dictionary instead of reading it is the main RAM win over the C++ engine:
//! pages of a language file stay in the page cache and can be evicted under
//! memory pressure, whereas the original `read()`-into-heap approach pins the
//! whole dictionary in the keyboard's anonymous RSS for the process lifetime.

use core::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::io::AsRawFd;
use std::ptr;

extern "C" {
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> i32;
}

const PROT_READ: i32 = 0x1;
const MAP_PRIVATE: i32 = 0x2;

/// Owns either a private read-only mapping or a heap copy of some bytes.
pub struct DataSource {
    ptr: *mut u8,
    len: usize,
    /// `Some` when the data could not be mapped and had to be copied.
    heap: Option<Vec<u8>>,
    /// Kept alive so the descriptor outlives the mapping.
    file: Option<File>,
}

// A dictionary is immutable once opened and is only ever read through `&self`,
// so sharing it between the IME thread and a suggestion worker is sound.
unsafe impl Send for DataSource {}
unsafe impl Sync for DataSource {}

impl DataSource {
    pub fn empty() -> DataSource {
        DataSource {
            ptr: ptr::null_mut(),
            len: 0,
            heap: Some(Vec::new()),
            file: None,
        }
    }

    /// Maps (or copies) `size` bytes starting at `offset` of `path`.
    ///
    /// `size == 0` means "until end of file", which is what the Java layer
    /// passes for the bundled, read-only dictionaries.
    pub fn open(path: &str, offset: u64, size: usize) -> Option<DataSource> {
        let mut file = File::open(path).ok()?;
        let file_len = file.metadata().ok()?.len();
        if offset > file_len {
            return None;
        }
        let available = (file_len - offset) as usize;
        let want = if size == 0 || size > available {
            available
        } else {
            size
        };
        if want == 0 {
            return Some(DataSource::empty());
        }

        // Only offsets that are a multiple of the page size can be mapped
        // directly; for anything else fall through to the copy path.
        if offset == 0 {
            let fd = file.as_raw_fd();
            let mapped = unsafe {
                mmap(
                    ptr::null_mut(),
                    want,
                    PROT_READ,
                    MAP_PRIVATE,
                    fd,
                    0,
                )
            };
            if !mapped.is_null() && mapped as isize != -1 {
                return Some(DataSource {
                    ptr: mapped as *mut u8,
                    len: want,
                    heap: None,
                    file: Some(file),
                });
            }
        }

        // Copy fallback (unaligned offsets, or mmap refused).
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut buffer = vec![0u8; want];
        let mut read_total = 0usize;
        while read_total < want {
            match file.read(&mut buffer[read_total..]) {
                Ok(0) => break,
                Ok(n) => read_total += n,
                Err(_) => return None,
            }
        }
        buffer.truncate(read_total);
        Some(DataSource {
            ptr: ptr::null_mut(),
            len: buffer.len(),
            heap: Some(buffer),
            file: None,
        })
    }

    pub fn from_vec(data: Vec<u8>) -> DataSource {
        DataSource {
            ptr: ptr::null_mut(),
            len: data.len(),
            heap: Some(data),
            file: None,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match &self.heap {
            Some(v) => v.as_slice(),
            None => {
                if self.ptr.is_null() || self.len == 0 {
                    &[]
                } else {
                    unsafe { core::slice::from_raw_parts(self.ptr as *const u8, self.len) }
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when the bytes are backed by the page cache rather than by private
    /// anonymous memory (used by the diagnostics screen / probe tool).
    pub fn is_mapped(&self) -> bool {
        self.heap.is_none()
    }
}

impl Default for DataSource {
    fn default() -> DataSource {
        DataSource::empty()
    }
}

impl Drop for DataSource {
    fn drop(&mut self) {
        if self.heap.is_none() && !self.ptr.is_null() && self.len > 0 {
            unsafe {
                munmap(self.ptr as *mut c_void, self.len);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

/// A `(offset, length)` window into a [`DataSource`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Buf {
    pub offset: usize,
    pub len: usize,
}

impl Buf {
    pub fn new(offset: usize, len: usize) -> Buf {
        Buf { offset, len }
    }

    pub fn empty() -> Buf {
        Buf { offset: 0, len: 0 }
    }

    pub fn get<'a>(&self, source: &'a [u8]) -> &'a [u8] {
        let end = self.offset.saturating_add(self.len);
        if self.offset >= source.len() || self.len == 0 {
            return &[];
        }
        let end = if end > source.len() { source.len() } else { end };
        &source[self.offset..end]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// File length in bytes, or 0 when unavailable.
pub fn file_len(path: &str) -> u64 {
    match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_backed_source_roundtrip() {
        let src = DataSource::from_vec(vec![1, 2, 3, 4]);
        assert_eq!(src.as_slice(), &[1, 2, 3, 4]);
        assert!(!src.is_mapped());
        let window = Buf::new(1, 2);
        assert_eq!(window.get(src.as_slice()), &[2, 3]);
    }

    #[test]
    fn empty_source_is_safe() {
        let src = DataSource::empty();
        assert!(src.as_slice().is_empty());
        assert_eq!(Buf::new(100, 10).get(src.as_slice()), &[] as &[u8]);
    }

    #[test]
    fn maps_this_test_binary() {
        // `/proc/self/exe` always exists on Android and on Linux hosts.
        let src = DataSource::open("/proc/self/exe", 0, 0);
        match src {
            Some(source) => {
                assert!(source.len() > 0);
                assert!(!source.as_slice().is_empty());
            }
            None => {}
        }
    }
}
