use nix::libc::memmem;
use std::borrow::Cow;
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

pub fn delete_subslices<'a>(data: &'a [u8], delete_sequences: &[&[u8]]) -> Cow<'a, [u8]> {
    if delete_sequences.is_empty() {
        return Cow::Borrowed(data);
    }

    let mut data = Vec::from(data);
    for seq in delete_sequences {
        while let Some(index) = find_subslice(&data, seq) {
            data.drain(index..index + seq.len());
        }
    }
    Cow::Owned(data)
}

#[cfg(test)]
mod tests {
    use crate::utils::delete_subslices;

    #[test]
    fn test_delete_subslices() {
        assert_eq!(delete_subslices(b"", &[]).as_ref(), b"");
        assert_eq!(delete_subslices(b"", &[b"foo"]).as_ref(), b"");
        assert_eq!(
            delete_subslices(b"aaabbbcccde", &[]).as_ref(),
            b"aaabbbcccde"
        );
        assert_eq!(
            delete_subslices(b"aaabbbcccde", &[b"aaaa"]).as_ref(),
            b"aaabbbcccde"
        );
        assert_eq!(
            delete_subslices(b"aaabbbcccde", &[b"aaa", b"ccc"]).as_ref(),
            b"bbbde"
        );
        assert_eq!(
            delete_subslices(b"aaabbbccccaade", &[b"ccc", b"aa"]).as_ref(),
            b"abbbcde"
        );
    }
}
