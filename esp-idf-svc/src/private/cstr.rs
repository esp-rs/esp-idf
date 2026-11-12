#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub use alloc::ffi::CString;

pub use core::ffi::{c_char, CStr};
use core::str::Utf8Error;

use crate::sys::{EspError, ESP_ERR_INVALID_SIZE};

pub fn set_str(buf: &mut [u8], s: &str) -> Result<(), EspError> {
    if s.len() >= buf.len() {
        return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
    }

    buf[..s.len()].copy_from_slice(s.as_bytes());
    buf[s.len()] = 0;

    Ok(())
}

pub fn set_str_no_termination_requirement(buf: &mut [u8], s: &str) -> Result<(), EspError> {
    if s.len() > buf.len() {
        return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
    }

    buf[..s.len()].copy_from_slice(s.as_bytes());
    if buf.len() != s.len() {
        buf[s.len()] = 0;
    }

    Ok(())
}

#[allow(clippy::unnecessary_cast)]
pub fn c_char_to_u8_slice_mut(s: &mut [c_char]) -> &mut [u8] {
    let s_ptr = unsafe { s.as_mut_ptr() as *mut u8 };
    unsafe { core::slice::from_raw_parts_mut(s_ptr, s.len()) }
}

pub unsafe fn from_cstr_ptr<'a>(ptr: *const c_char) -> &'a str {
    CStr::from_ptr(ptr).to_str().unwrap()
}

pub fn from_cstr_fallible(buf: &[u8]) -> Result<&str, Utf8Error> {
    // We have to find the first '\0' ourselves, because the passed buffer might
    // be wider than the ASCIIZ string it contains
    let len = buf.iter().position(|e| *e == 0).unwrap() + 1;

    unsafe { CStr::from_bytes_with_nul_unchecked(&buf[0..len]) }.to_str()
}

pub fn from_cstr(buf: &[u8]) -> &str {
    from_cstr_fallible(buf).unwrap()
}

/// Convert buffer of characters to heapless string, allowing either
/// the full buffer (without terminating 0) or just a part of it (terminated by 0)
pub fn array_to_heapless_string_failible<const N: usize>(
    arr: [u8; N],
) -> Result<heapless::String<N>, Utf8Error> {
    let len = arr.iter().position(|e| *e == 0).unwrap_or(N);
    heapless::String::from_utf8(heapless::Vec::from_slice(&arr[0..len]).unwrap())
}

pub fn array_to_heapless_string<const N: usize>(arr: [u8; N]) -> heapless::String<N> {
    array_to_heapless_string_failible(arr).unwrap()
}

#[cfg(feature = "alloc")]
pub struct RawCstrs(alloc::vec::Vec<CString>);

#[cfg(feature = "alloc")]
impl RawCstrs {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self(alloc::vec::Vec::new())
    }

    #[allow(dead_code)]
    pub fn as_ptr(&mut self, s: impl AsRef<str>) -> Result<*const c_char, crate::sys::EspError> {
        let cs = to_cstring_arg(s.as_ref())?;

        let cstr_ptr = cs.as_ptr();

        self.0.push(cs);

        Ok(cstr_ptr)
    }

    #[allow(dead_code)]
    pub fn as_nptr<S>(&mut self, s: Option<S>) -> Result<*const c_char, crate::sys::EspError>
    where
        S: AsRef<str>,
    {
        s.map(|s| self.as_ptr(s)).unwrap_or(Ok(core::ptr::null()))
    }
}

#[cfg(feature = "alloc")]
impl Default for RawCstrs {
    fn default() -> Self {
        RawCstrs::new()
    }
}

#[cfg(feature = "alloc")]
pub fn nul_to_invalid_arg(_err: alloc::ffi::NulError) -> crate::sys::EspError {
    crate::sys::EspError::from_non_zero(
        core::num::NonZeroI32::new(crate::sys::ESP_ERR_INVALID_ARG).unwrap(),
    )
}

#[cfg(feature = "alloc")]
pub fn to_cstring_arg(value: &str) -> Result<CString, crate::sys::EspError> {
    CString::new(value).map_err(nul_to_invalid_arg)
}

/// str to cstr, will be truncated if str is larger than buf.len() - 1
///
/// # Panics
///
/// * Panics if buffer is empty.
pub fn cstr_from_str_truncating<'a>(rust_str: &str, buf: &'a mut [u8]) -> &'a CStr {
    assert!(!buf.is_empty());

    let max_str_size = buf.len() - 1; // account for NUL
    let truncated_str = &rust_str[..max_str_size.min(rust_str.len())];
    buf[..truncated_str.len()].copy_from_slice(truncated_str.as_bytes());
    buf[truncated_str.len()] = b'\0';

    CStr::from_bytes_with_nul(&buf[..truncated_str.len() + 1]).unwrap()
}

/// Convert slice of rust strs to NULL-terminated fixed size array of c string pointers
///
/// # Panics
///
/// * Panics if cbuf is empty.
/// * Panics if N is <= 1
#[cfg_attr(not(feature = "alloc"), allow(unused))]
pub fn cstr_arr_from_str_slice<const N: usize>(
    rust_strs: &[&str],
    mut cbuf: &mut [u8],
) -> Result<[*const c_char; N], EspError> {
    assert!(N > 1);
    assert!(!cbuf.is_empty());

    // ensure last element stays NULL
    if rust_strs.len() > N - 1 {
        return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
    }

    let mut cstrs = [core::ptr::null(); N];

    for (i, s) in rust_strs.iter().enumerate() {
        let max_str_size = cbuf.len() - 1; // account for NUL
        if s.len() > max_str_size {
            return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
        }
        cbuf[..s.len()].copy_from_slice(s.as_bytes());
        cbuf[s.len()] = b'\0';
        let cstr = CStr::from_bytes_with_nul(&cbuf[..s.len() + 1]).unwrap();
        cstrs[i] = cstr.as_ptr();

        cbuf = &mut cbuf[s.len() + 1..];
    }

    Ok(cstrs)
}

#[cfg(test)]
mod tests {
    use super::{cstr_arr_from_str_slice, cstr_from_str_truncating, CStr};

    #[test]
    fn cstr_from_str_happy() {
        let mut same_size = [0u8; 6];
        let hello = cstr_from_str_truncating("Hello", &mut same_size);
        assert_eq!(hello.to_bytes(), b"Hello");

        let mut larger = [0u8; 42];
        let hello = cstr_from_str_truncating("Hello", &mut larger);
        assert_eq!(hello.to_bytes(), b"Hello");
    }

    #[test]
    fn cstr_from_str_unhappy() {
        let mut smaller = [0u8; 6];
        let hello = cstr_from_str_truncating("Hello World", &mut smaller);
        assert_eq!(hello.to_bytes(), b"Hello");
    }

    #[test]
    fn cstr_arr_happy() {
        let mut same_size = [0u8; 13];
        let hello = cstr_arr_from_str_slice::<3>(&["Hello", "World"], &mut same_size).unwrap();
        assert_eq!(unsafe { CStr::from_ptr(hello[0]) }.to_bytes(), b"Hello");
        assert_eq!(unsafe { CStr::from_ptr(hello[1]) }.to_bytes(), b"World");
        assert_eq!(hello[2], core::ptr::null());
    }

    #[test]
    #[should_panic]
    fn cstr_arr_unhappy_n1() {
        let mut cbuf = [0u8; 25];
        let _ = cstr_arr_from_str_slice::<1>(&["Hello"], &mut cbuf);
    }

    #[test]
    fn cstr_arr_unhappy_n_too_small() {
        let mut cbuf = [0u8; 25];
        assert!(cstr_arr_from_str_slice::<2>(&["Hello", "World"], &mut cbuf).is_err());
    }

    #[test]
    #[should_panic]
    fn cstr_arr_unhappy_cbuf_too_small() {
        let mut cbuf = [0u8; 12];
        assert!(cstr_arr_from_str_slice::<3>(&["Hello", "World"], &mut cbuf).is_err());
    }
}
