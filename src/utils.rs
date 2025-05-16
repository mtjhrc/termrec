use nix::libc::memmem;
use std::ffi::c_void;

pub fn find_subslice(heysstack: &[u8], needle: &[u8]) -> Option<usize> {
    if heysstack.is_empty() {
        return None;
    }
    if needle.is_empty() {
        panic!("Empty substring to search");
    }

    // SAFETY: Safe, the heystack and needle pointers have to be valid pointers because they are
    // constructed from references. The returned pointer is only used to calculate an index, usage
    // of which would be bounds-checked anyway.
    unsafe {
        let heystack_ptr = heysstack.as_ptr() as *const c_void;
        let needle_ptr = needle.as_ptr() as *const c_void;

        let ptr = memmem(heystack_ptr, heysstack.len(), needle_ptr, needle.len());
        if ptr.is_null() {
            None
        } else {
            let index: usize = ptr
                .offset_from(heystack_ptr)
                .try_into()
                .expect("memmem returned pointer located before the start of the string");
            assert!(index < heysstack.len());
            Some(index)
        }
    }
}
