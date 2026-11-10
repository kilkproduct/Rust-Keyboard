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
    fn madvise(addr: *mut c_void, length: usize, advice: i32) -> i32;
    fn sysconf(name: i32) -> i64;
}

const PROT_READ: i32 = 0x1;
const MAP_PRIVATE: i32 = 0x2;

/// Linux/Android `_SC_PAGESIZE`.
const SC_PAGESIZE: i32 = 30;

/// Access patterns for [`DataSource::advise`], from `madvise(2)`.
pub const MADV_NORMAL: i32 = 0;
pub const MADV_RANDOM: i32 = 1;
pub const MADV_SEQUENTIAL: i32 = 2;
pub const MADV_WILLNEED: i32 = 3;

fn page_size() -> usize {
    // Both Android and every supported host answer 4096 here; the fallback only
    // matters if the call is ever made without a working libc.
    let size = unsafe { sysconf(SC_PAGESIZE) };
    if size >= 512 {
        size as usize
    } else {
        4096
    }
}

/// Owns either a private read-only mapping or a heap copy of some bytes.
pub struct DataSource {
    ptr: *mut u8,
    len: usize,
    /// `Some` when the data could not be mapped and had to be copied.
    heap: Option<Vec<u8>>,
    /// Base address and length of the mapping itself, which starts *before*
    /// `ptr` when the dictionary was opened at an unaligned offset. The
    /// descriptor is not kept: `mmap` keeps its own reference to the file.
    map_ptr: *mut u8,
    map_len: usize,
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
            map_ptr: ptr::null_mut(),
            map_len: 0,
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

        // `mmap` only accepts a page aligned file offset, so map from the page
        // that contains `offset` and hand out the window inside it. A dictionary
        // that starts at a non-zero offset (a resource inside the APK, say) used
        // to take the copy path below and put the whole file on the heap.
        let page = page_size();
        let aligned = offset - (offset % page as u64);
        let delta = (offset - aligned) as usize;
        let map_len = want + delta;
        {
            let fd = file.as_raw_fd();
            let mapped = unsafe {
                mmap(
                    ptr::null_mut(),
                    map_len,
                    PROT_READ,
                    MAP_PRIVATE,
                    fd,
                    aligned as i64,
                )
            };
            if !mapped.is_null() && mapped as isize != -1 {
                let base = mapped as *mut u8;
                let source = DataSource {
                    // `delta` is inside the mapping, so this cannot overflow.
                    ptr: unsafe { base.add(delta) },
                    len: want,
                    heap: None,
                    map_ptr: base,
                    map_len,
                };
                // The trie is walked in near random order: tell the kernel not
                // to read ahead, and pull in the first page immediately so the
                // open path does not fault in the header twice.
                source.advise(MADV_RANDOM);
                source.willneed(0, 64 * 1024);
                // The descriptor is not needed once the mapping exists.
                drop(file);
                return Some(source);
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
            map_ptr: ptr::null_mut(),
            map_len: 0,
        })
    }

    pub fn from_vec(data: Vec<u8>) -> DataSource {
        DataSource {
            ptr: ptr::null_mut(),
            len: data.len(),
            heap: Some(data),
            map_ptr: ptr::null_mut(),
            map_len: 0,
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

    /// `madvise(2)` hint over the whole mapping. No-op for heap backed data.
    pub fn advise(&self, advice: i32) -> bool {
        if self.heap.is_some() || self.map_ptr.is_null() || self.map_len == 0 {
            return false;
        }
        unsafe { madvise(self.map_ptr as *mut c_void, self.map_len, advice) == 0 }
    }

    /// Asks the kernel to read `len` bytes at `offset` of the window ahead of
    /// the first access, which turns a page fault per node into one request.
    pub fn willneed(&self, offset: usize, len: usize) -> bool {
        if self.heap.is_some() || self.ptr.is_null() || len == 0 || offset >= self.len {
            return false;
        }
        let available = self.len - offset;
        let len = len.min(available);
        let start = unsafe { self.ptr.add(offset) };
        let page = page_size();
        // `madvise` wants page aligned addresses; drop the misalignment.
        let misalign = (start as usize) % page;
        let base = unsafe { start.sub(misalign) };
        let mut length = len + misalign;
        length -= length % page;
        if length == 0 {
            length = page;
        }
        unsafe { madvise(base as *mut c_void, length, MADV_WILLNEED) == 0 }
    }
}

impl Default for DataSource {
    fn default() -> DataSource {
        DataSource::empty()
    }
}

impl Drop for DataSource {
    fn drop(&mut self) {
        if self.heap.is_none() && !self.map_ptr.is_null() && self.map_len > 0 {
            unsafe {
                munmap(self.map_ptr as *mut c_void, self.map_len);
            }
        }
        self.ptr = ptr::null_mut();
        self.map_ptr = ptr::null_mut();
        self.map_len = 0;
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
    fn page_size_is_sane() {
        let page = page_size();
        assert!(page >= 512 && page.is_power_of_two(), "page size {}", page);
    }

    #[test]
    fn unaligned_offset_is_mapped_not_copied() {
        // The window must be mapped from the page that contains `offset`, so a
        // dictionary stored at a non-zero offset stays in the page cache
        // instead of being copied onto the heap.
        let path = std::env::temp_dir().join(format!("kb-mmap-{}", std::process::id()));
        let path = path.to_string_lossy().to_string();
        let mut data = Vec::new();
        for index in 0..(page_size() * 2) {
            data.push((index % 251) as u8);
        }
        std::fs::write(&path, &data).expect("write");

        for offset in [1usize, 3, 100, page_size() - 1, page_size(), page_size() + 7] {
            let source = DataSource::open(&path, offset as u64, 0).expect("opens");
            assert!(source.is_mapped(), "offset {} was copied", offset);
            assert_eq!(source.len(), data.len() - offset);
            let slice = source.as_slice();
            assert_eq!(slice[0], data[offset], "offset {}", offset);
            assert_eq!(slice[slice.len() - 1], data[data.len() - 1]);
            // Hints must be cheap and must not fault.
            let _ = source.advise(MADV_RANDOM);
            let _ = source.willneed(0, 4096);
            let _ = source.willneed(source.len(), 4096);
        }

        // An explicit size still honours the window.
        let source = DataSource::open(&path, 5, 32).expect("opens");
        assert_eq!(source.as_slice(), &data[5..37]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn window_past_the_end_is_clamped() {
        let path = std::env::temp_dir().join(format!("kb-mmap-short-{}", std::process::id()));
        let path = path.to_string_lossy().to_string();
        std::fs::write(&path, [1u8, 2, 3, 4]).expect("write");

        let source = DataSource::open(&path, 2, 0).expect("opens");
        assert_eq!(source.as_slice(), &[3, 4]);

        // A size that runs past the end is clamped to the end, not rejected.
        let source = DataSource::open(&path, 2, 4096).expect("opens");
        assert_eq!(source.as_slice(), &[3, 4]);

        // Opening at the very end yields an empty window; past it, nothing.
        let source = DataSource::open(&path, 4, 0).expect("opens");
        assert!(source.as_slice().is_empty());
        assert!(DataSource::open(&path, 5, 0).is_none());
        assert!(DataSource::open(&path, 99, 0).is_none());

        let _ = std::fs::remove_file(&path);
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
