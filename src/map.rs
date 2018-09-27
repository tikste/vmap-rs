use std::fmt;
use std::slice;
use std::fs::File;
use std::io::{Result, Error, ErrorKind};
use std::ops::{Deref, DerefMut};

use ::{PageSize, Protect, Flush};
use ::os::{map_file, map_anon, unmap, protect, flush};



/// Allocation of one or more read-only sequential pages.
///
/// # Example
///
/// ```
/// # extern crate vmap;
/// use vmap::Map;
/// use std::fs::OpenOptions;
///
/// # fn main() -> std::io::Result<()> {
/// let file = OpenOptions::new().read(true).open("src/lib.rs")?;
/// let page = Map::file(&file, 0, 256)?;
/// assert_eq!(b"fast and safe memory-mapped IO", &page[33..63]);
/// # Ok(())
/// # }
pub struct Map {
    base: MapMut,
}

fn file_checked(f: &File, off: usize, len: usize, prot: Protect) -> Result<*mut u8> {
    if f.metadata()?.len() < (off+len) as u64 {
        Err(Error::new(ErrorKind::InvalidInput, "map range not in file"))
    } else {
        unsafe { file_unchecked(f, off, len, prot) }
    }
}

unsafe fn file_unchecked(f: &File, off: usize, len: usize, prot: Protect) -> Result<*mut u8> {
    let sz = PageSize::new();
    let roff = sz.truncate(off);
    let rlen = sz.round(len + (off - roff));
    let ptr = map_file(f, roff, rlen, prot)?;
    Ok(ptr.offset((off - roff) as isize))
}

impl Map {
    /// Create a new map object from a range of a file.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate vmap;
    /// use std::fs::OpenOptions;
    /// use vmap::Map;
    ///
    /// # fn main() -> std::io::Result<()> {
    /// let file = OpenOptions::new().read(true).open("src/lib.rs")?;
    /// let map = Map::file(&file, 0, 256)?;
    /// assert_eq!(map.is_empty(), false);
    /// assert_eq!(b"fast and safe memory-mapped IO", &map[33..63]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn file(f: &File, offset: usize, length: usize) -> Result<Self> {
        let ptr = file_checked(f, offset, length, Protect::ReadOnly)?;
        Ok(unsafe { Self::from_ptr(ptr, length) })
    }

    /// Create a new map object from a range of a file without bounds checking.
    ///
    /// # Safety
    ///
    /// This does not verify that the requsted range is valid for the file.
    /// This can be useful in a few scenarios:
    /// 1. When the range is already known to be valid.
    /// 2. When a valid sub-range is known and not exceeded.
    /// 3. When the range will become valid and is not used until then.
    pub unsafe fn file_unchecked(f: &File, offset: usize, length: usize) -> Result<Self> {
        let ptr = file_unchecked(f, offset, length, Protect::ReadOnly)?;
        Ok(Self::from_ptr(ptr, length))
    }

    /// Constructs a new page sequence from an existing mapping.
    ///
    /// # Safety
    ///
    /// This does not know or care if `ptr` or `len` are valid. That is,
    /// it may be null, not at a proper page boundary, point to a size
    /// different from `len`, or worse yet, point to properly mapped pointer
    /// from some other allocation system.
    ///
    /// Generally don't use this unless you are entirely sure you are
    /// doing so correctly.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate vmap;
    /// use vmap::{Map, Protect};
    /// use std::fs::OpenOptions;
    ///
    /// # fn main() -> std::io::Result<()> {
    /// let file = OpenOptions::new().read(true).open("src/lib.rs")?;
    /// let page = unsafe {
    ///     let len = vmap::page_size();
    ///     let ptr = vmap::os::map_file(&file, 0, len, Protect::ReadOnly)?;
    ///     Map::from_ptr(ptr, len)
    /// };
    /// assert_eq!(b"fast and safe memory-mapped IO", &page[33..63]);
    /// # Ok(())
    /// # }
    /// ```
    pub unsafe fn from_ptr(ptr: *mut u8, len: usize) -> Self {
        Self { base: MapMut::from_ptr(ptr, len) }
    }

    pub fn make_mut(self) -> Result<MapMut> {
        unsafe {
            let (ptr, len) = PageSize::new().bounds(self.base.ptr, self.base.len);
            protect(ptr, len, Protect::ReadWrite)?;
        }
        Ok(self.base)
    }
}

impl Deref for Map {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] { self.base.deref() }
}

impl AsRef<[u8]> for Map {
    #[inline]
    fn as_ref(&self) -> &[u8] { self.deref() }
}

impl fmt::Debug for Map {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Map")
            .field("ptr", &self.base.ptr)
            .field("len", &self.base.len)
            .finish()
    }
}



/// Allocation of one or more read-write sequential pages.
#[derive(Debug)]
pub struct MapMut {
    ptr: *mut u8,
    len: usize,
}

impl MapMut {
    /// Create a new anonymous mapping at least as large as the hint.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate vmap;
    /// use vmap::MapMut;
    /// use std::io::Write;
    ///
    /// # fn main() -> std::io::Result<()> {
    /// let mut map = MapMut::new(200)?;
    /// {
    ///     let mut data = &mut map[..];
    ///     assert!(data.len() >= 200);
    ///     data.write_all(b"test")?;
    /// }
    /// assert_eq!(b"test", &map[..4]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(hint: usize) -> Result<Self> {
        unsafe {
            let len = PageSize::new().round(hint);
            let ptr = map_anon(len)?;
            Ok(Self::from_ptr(ptr, len))
        }
    }

    /// Create a new mutable map object from a range of a file.
    pub fn file(f: &File, offset: usize, length: usize) -> Result<Self> {
        let ptr = file_checked(f, offset, length, Protect::ReadWrite)?;
        Ok(unsafe { Self::from_ptr(ptr, length) })
    }

    /// Create a new mutable map object from a range of a file without bounds
    /// checking.
    ///
    /// # Safety
    ///
    /// This does not verify that the requsted range is valid for the file.
    /// This can be useful in a few scenarios:
    /// 1. When the range is already known to be valid.
    /// 2. When a valid sub-range is known and not exceeded.
    /// 3. When the range will become valid and is not used until then.
    pub unsafe fn file_unchecked(f: &File, offset: usize, length: usize) -> Result<Self> {
        let ptr = file_unchecked(f, offset, length, Protect::ReadWrite)?;
        Ok(Self::from_ptr(ptr, length))
    }

    /// Create a new private map object from a range of a file.
    ///
    /// Initially, the mapping will be shared with other processes, but writes
    /// will be kept private.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate vmap;
    /// use vmap::MapMut;
    /// use std::io::Write;
    /// use std::fs::OpenOptions;
    ///
    /// # fn main() -> std::io::Result<()> {
    /// let file = OpenOptions::new().read(true).open("src/lib.rs")?;
    /// let mut map = MapMut::copy(&file, 33, 30)?;
    /// assert_eq!(map.is_empty(), false);
    /// assert_eq!(b"fast and safe memory-mapped IO", &map[..]);
    /// {
    ///     let mut data = &mut map[..];
    ///     data.write_all(b"slow")?;
    /// }
    /// assert_eq!(b"slow and safe memory-mapped IO", &map[..]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn copy(f: &File, offset: usize, length: usize) -> Result<Self> {
        let ptr = file_checked(f, offset, length, Protect::ReadCopy)?;
        Ok(unsafe { Self::from_ptr(ptr, length) })
    }

    /// Create a new private map object from a range of a file without bounds checking.
    ///
    /// Initially, the mapping will be shared with other processes, but writes
    /// will be kept private.
    ///
    /// # Safety
    ///
    /// This does not verify that the requsted range is valid for the file.
    /// This can be useful in a few scenarios:
    /// 1. When the range is already known to be valid.
    /// 2. When a valid sub-range is known and not exceeded.
    /// 3. When the range will become valid before any write occurs.
    pub unsafe fn copy_unchecked(f: &File, offset: usize, length: usize) -> Result<Self> {
        let ptr = file_unchecked(f, offset, length, Protect::ReadCopy)?;
        Ok(Self::from_ptr(ptr, length))
    }

    pub unsafe fn from_ptr(ptr: *mut u8, len: usize) -> Self {
        Self { ptr: ptr, len: len }
    }

    pub fn make_read_only(self) -> Result<Map> {
        unsafe {
            let (ptr, len) = PageSize::new().bounds(self.ptr, self.len);
            protect(ptr, len, Protect::ReadWrite)?;
        }
        Ok(Map { base: self })
    }

    pub fn flush(&self, file: &File, mode: Flush) -> Result<()> {
        unsafe {
            let (ptr, len) = PageSize::new().bounds(self.ptr, self.len);
            flush(ptr, file, len, mode)
        }
    }
}

impl Drop for MapMut {
    fn drop(&mut self) {
        unsafe {
            let (ptr, len) = PageSize::new().bounds(self.ptr, self.len);
            unmap(ptr, len).unwrap_or_default();
        }
    }
}

impl Deref for MapMut {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for MapMut {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl AsRef<[u8]> for MapMut {
    #[inline]
    fn as_ref(&self) -> &[u8] { self.deref() }
}

impl AsMut<[u8]> for MapMut {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] { self.deref_mut() }
}

