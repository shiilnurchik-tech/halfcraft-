use super::*;
use std::{ffi::c_void, ptr::NonNull};

type Handle = *mut c_void;
const FILE_MAP_READ: u32 = 4;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn MapViewOfFile(
        mapping: Handle,
        access: u32,
        high: u32,
        low: u32,
        bytes: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(base: *const c_void) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetTickCount64() -> u64;
}

pub struct Mapping {
    handle: Handle,
    base: NonNull<u32>,
}
// A mapping/handle may cross threads. Access is read-only, the OS keeps the view
// alive until Drop, and native hooks serialize ownership behind a Mutex.
unsafe impl Send for Mapping {}

impl Mapping {
    pub fn open() -> Result<Self, LinkError> {
        let name: Vec<u16> = MAPPING_NAME.encode_utf16().chain(Some(0)).collect();
        // SAFETY: nul-terminated name, no borrowed memory retained by Win32.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(LinkError::Unavailable);
        }
        // Map only the header and host state; never create a replacement mapping.
        let base = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, VIEW_BYTES) };
        let Some(base) = NonNull::new(base.cast::<u32>()) else {
            unsafe {
                CloseHandle(handle);
            }
            return Err(LinkError::Unavailable);
        };
        let mapping = Self { handle, base };
        mapping.read()?;
        Ok(mapping)
    }
    pub fn read(&self) -> Result<HostSnapshot, LinkError> {
        // GetTickCount64 is the same clock Source writes to the shared header.
        let now = unsafe { GetTickCount64() };
        snapshot(
            |at| {
                debug_assert!(at % 4 == 0 && at + 4 <= VIEW_BYTES);
                // SAFETY: offsets originate from the checked-in protocol generator;
                // the view is page aligned, mapped read-only and owned by self.
                unsafe { self.base.as_ptr().add(at / 4).read_volatile() }
            },
            now,
        )
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.base.as_ptr().cast());
            CloseHandle(self.handle);
        }
    }
}
