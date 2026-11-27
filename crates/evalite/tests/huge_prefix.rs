//! A stored program whose length prefix claims a huge number of entries
//! fails to load without allocating for them. It is its own test binary
//! so it can count allocations.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use evalite::{Environment, Program, Type};

struct Counting;

static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LARGEST.fetch_max(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// A varint of `n`, 7 bits a byte, low bits first.
fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();

    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;

        if n == 0 {
            out.push(byte);
            return out;
        }

        out.push(byte | 0x80);
    }
}

#[test]
fn huge_length_prefix() {
    let mut env = Environment::new();
    env.value("i", Type::int()).unwrap();
    let bytes = evalite::compile("i", &env).unwrap().to_bytes().unwrap();
    assert!(Program::from_bytes(&bytes).is_ok());

    // The stored form starts with its field count, then each field's tag
    // and value: `format` (tag 0) and `types` (tag 1), whose first byte is
    // its length. Every length prefix of the program is tried.
    assert_eq!(&bytes[..4], &[8, 0, 4, 1]);

    for at in 0..bytes.len() {
        for claimed in [u64::from(u32::MAX), u64::MAX >> 1, u64::MAX] {
            let mut tampered = bytes[..at].to_vec();
            tampered.extend(varint(claimed));
            tampered.extend(&bytes[at + 1..]);

            LARGEST.store(0, Ordering::Relaxed);
            let result = Program::from_bytes(&tampered);
            let largest = LARGEST.load(Ordering::Relaxed);

            // musli caps what it reserves ahead of a length at 1 MiB.
            assert!(
                largest <= 1 << 20,
                "byte {at} claiming {claimed} allocated {largest} bytes"
            );

            if at == 4 {
                let error = result.expect_err("a huge length prefix decoded");
                assert_eq!(error.to_string(), "the bytes do not decode", "{error:?}");
            }
        }
    }
}
