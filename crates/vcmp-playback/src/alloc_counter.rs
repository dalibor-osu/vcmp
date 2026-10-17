//! A global allocator for the test build that counts allocations and frees made on
//! a thread while it is being watched, to prove the audio callback makes none.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

pub(crate) struct CountingAllocator;

fn note() {
    // `try_with`: allocations also happen while thread-locals are torn down.
    let _ = WATCHING.try_with(|watching| {
        if watching.get() {
            let _ = COUNT.try_with(|count| count.set(count.get() + 1));
        }
    });
}

// SAFETY: every call is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Runs `f` and returns how many allocations, reallocations and frees it made on
/// this thread.
pub(crate) fn allocations_in(f: impl FnOnce()) -> usize {
    COUNT.with(|count| count.set(0));
    WATCHING.with(|watching| watching.set(true));
    f();
    WATCHING.with(|watching| watching.set(false));
    COUNT.with(|count| count.get())
}

#[cfg(test)]
mod tests {
    use super::allocations_in;

    #[test]
    fn the_counter_sees_allocations() {
        let count = allocations_in(|| drop(std::hint::black_box(vec![1u8; 64])));
        assert_eq!(count, 2, "one allocation, one free");
        assert_eq!(allocations_in(|| {}), 0);
    }
}
