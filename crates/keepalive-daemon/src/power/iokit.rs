use std::ffi::{CString, c_void};

type CFStringRef = *const c_void;
type CFAllocatorRef = *const c_void;
type IOPMAssertionID = u32;
type IOReturn = i32;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
const K_IO_RETURN_SUCCESS: IOReturn = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        assertion_level: u32,
        assertion_name: CFStringRef,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const i8,
        encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

fn cf_string(s: &str) -> Option<CFStringRef> {
    let c = CString::new(s).ok()?;
    let cf = unsafe {
        CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8)
    };
    if cf.is_null() { None } else { Some(cf) }
}

/// RAII handle on a PreventUserIdleSystemSleep power assertion.
/// Dropping it releases the assertion, so a daemon crash can never leave
/// the machine permanently wired.
pub struct WakeAssertion {
    id: IOPMAssertionID,
}

impl WakeAssertion {
    pub fn new(reason: &str) -> Option<Self> {
        let assertion_type = cf_string("PreventUserIdleSystemSleep")?;
        let name = cf_string(reason)?;
        let mut id: IOPMAssertionID = 0;
        let ret = unsafe {
            IOPMAssertionCreateWithName(assertion_type, K_IOPM_ASSERTION_LEVEL_ON, name, &mut id)
        };
        unsafe {
            CFRelease(assertion_type);
            CFRelease(name);
        }
        (ret == K_IO_RETURN_SUCCESS).then_some(Self { id })
    }
}

impl Drop for WakeAssertion {
    fn drop(&mut self) {
        unsafe {
            IOPMAssertionRelease(self.id);
        }
    }
}
