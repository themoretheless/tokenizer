//! Reservation primitive for the pending managed-value storage migration.
//! Counts requested bytes, not allocator overhead or resident memory.
use std::alloc::{Layout, alloc, alloc_zeroed, dealloc};
use std::ptr::NonNull;
use std::{cell::Cell, rc::Rc};

#[derive(Debug)]
struct Ledger {
    limit: usize,
    live: Cell<usize>,
    peak: Cell<usize>,
}

#[derive(Clone, Debug)]
struct Budget(Rc<Ledger>);

#[derive(Debug, PartialEq, Eq)]
struct LimitExceeded;

/// A unique reservation must travel with the allocation it accounts for.
/// Sharing the allocation shares its reservation; copying it reserves again.
#[derive(Debug)]
struct Reservation {
    ledger: Rc<Ledger>,
    bytes: usize,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self(Rc::new(Ledger {
            limit,
            live: Cell::new(0),
            peak: Cell::new(0),
        }))
    }

    fn reserve(&self, bytes: usize) -> Result<Reservation, LimitExceeded> {
        let mut reservation = Reservation {
            ledger: self.0.clone(),
            bytes: 0,
        };
        reservation.resize(bytes)?;
        Ok(reservation)
    }
}

impl Reservation {
    /// Grow before allocating. If allocation fails, restore the previous size.
    /// Shrink only after the corresponding storage has been released.
    fn resize(&mut self, bytes: usize) -> Result<(), LimitExceeded> {
        let other = self.ledger.live.get() - self.bytes;
        let next = other.checked_add(bytes).ok_or(LimitExceeded)?;
        if next > self.ledger.limit {
            return Err(LimitExceeded);
        }
        self.ledger.live.set(next);
        self.ledger.peak.set(self.ledger.peak.get().max(next));
        self.bytes = bytes;
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.live.set(self.ledger.live.get() - self.bytes);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AllocationError {
    Limit,
    Capacity,
    Allocator,
}

/// Owns raw storage, independently of element destruction. This field's Drop
/// still runs if an element destructor unwinds while Slots is being dropped.
#[derive(Debug)]
struct Storage<T> {
    pointer: NonNull<T>,
    layout: Layout,
    reservation: Reservation,
}

impl<T> Drop for Storage<T> {
    fn drop(&mut self) {
        if self.layout.size() != 0 {
            // SAFETY: this uniquely owned pointer was allocated with this layout.
            unsafe { dealloc(self.pointer.as_ptr().cast(), self.layout) };
        }
    }
}

#[derive(Debug)]
struct Slots<T> {
    storage: Storage<T>,
    capacity: usize,
    length: usize,
}

impl<T> Slots<T> {
    fn new(budget: &Budget, capacity: usize) -> Result<Self, AllocationError> {
        let layout = Layout::array::<T>(capacity).map_err(|_| AllocationError::Capacity)?;
        let reservation = budget
            .reserve(layout.size())
            .map_err(|_| AllocationError::Limit)?;
        let pointer = if layout.size() == 0 {
            NonNull::dangling()
        } else {
            // SAFETY: nonzero validated layout. No element is read until initialized.
            NonNull::new(unsafe { alloc(layout) }.cast::<T>()).ok_or(AllocationError::Allocator)?
        };
        Ok(Self {
            storage: Storage {
                pointer,
                layout,
                reservation,
            },
            capacity,
            length: 0,
        })
    }

    fn as_slice(&self) -> &[T] {
        // SAFETY: exactly the prefix `length` is initialized and owned by self.
        unsafe { std::slice::from_raw_parts(self.storage.pointer.as_ptr(), self.length) }
    }

    fn grow(&mut self, capacity: usize) -> Result<(), AllocationError> {
        if capacity <= self.capacity {
            return Ok(());
        }
        let mut replacement =
            Self::new(&Budget(self.storage.reservation.ledger.clone()), capacity)?;
        // SAFETY: disjoint allocations hold at least length slots. This moves
        // initialized elements without cloning. No user code runs before the
        // old length is cleared, so each element will be dropped only once.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.storage.pointer.as_ptr(),
                replacement.storage.pointer.as_ptr(),
                self.length,
            )
        };
        replacement.length = self.length;
        self.length = 0;
        *self = replacement;
        Ok(())
    }

    fn push(&mut self, value: T) -> Result<(), AllocationError> {
        if self.length == self.capacity {
            let next = self
                .capacity
                .checked_mul(2)
                .filter(|n| *n > self.capacity)
                .or_else(|| self.capacity.checked_add(1))
                .ok_or(AllocationError::Capacity)?;
            self.grow(next)?;
        }
        // SAFETY: this slot lies in spare capacity and is currently uninitialized.
        unsafe { self.storage.pointer.as_ptr().add(self.length).write(value) };
        self.length += 1;
        Ok(())
    }
}

impl<T> Drop for Slots<T> {
    fn drop(&mut self) {
        // SAFETY: only the initialized prefix is dropped. Storage is freed by
        // its own destructor afterwards, including during unwinding.
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.storage.pointer.as_ptr(),
                self.length,
            ))
        };
    }
}

#[test]
fn typed_storage_moves_values_and_releases_nested_allocations() {
    let slot_size = std::mem::size_of::<Bytes>();
    let budget = Budget::new(slot_size * 3 + 8);
    let mut values = Slots::new(&budget, 1).unwrap();
    values
        .push(Bytes::from_slice(&budget, b"first").unwrap())
        .unwrap();
    values
        .push(Bytes::from_slice(&budget, b"two").unwrap())
        .unwrap();
    assert_eq!(values.as_slice()[0].as_slice(), b"first");
    assert_eq!(values.as_slice()[1].as_slice(), b"two");
    assert_eq!(budget.0.live.get(), slot_size * 2 + 8);
    assert_eq!(budget.0.peak.get(), slot_size * 3 + 8);
    assert_eq!(values.grow(4), Err(AllocationError::Limit));
    assert_eq!(values.as_slice().len(), 2);
    drop(values);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn typed_storage_handles_alignment_zero_size_and_overflow() {
    #[repr(align(64))]
    struct Aligned(u8);
    let budget = Budget::new(64);
    let mut aligned = Slots::new(&budget, 1).unwrap();
    aligned.push(Aligned(7)).unwrap();
    assert_eq!(aligned.as_slice().as_ptr() as usize % 64, 0);
    assert_eq!(aligned.as_slice()[0].0, 7);
    drop(aligned);
    let mut empty = Slots::new(&budget, 0).unwrap();
    for _ in 0..100 {
        empty.push(()).unwrap();
    }
    assert_eq!(empty.as_slice().len(), 100);
    assert_eq!(budget.0.live.get(), 0);
    assert!(matches!(
        Slots::<u64>::new(&budget, usize::MAX),
        Err(AllocationError::Capacity)
    ));
}

#[test]
fn typed_storage_releases_reservation_when_element_drop_unwinds() {
    struct Element {
        drops: Rc<Cell<usize>>,
        panic: bool,
    }
    impl Drop for Element {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
            assert!(!self.panic, "test destructor panic");
        }
    }
    let budget = Budget::new(1024);
    let drops = Rc::new(Cell::new(0));
    let mut values = Slots::new(&budget, 2).unwrap();
    values
        .push(Element {
            drops: drops.clone(),
            panic: true,
        })
        .unwrap();
    values
        .push(Element {
            drops: drops.clone(),
            panic: false,
        })
        .unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(values))).is_err());
    assert_eq!(drops.get(), 2);
    assert_eq!(budget.0.live.get(), 0);
}

/// Exact-sized, initialized byte storage. No infallible Clone implementation:
/// a deep copy must acquire its own reservation before allocating.
#[derive(Debug)]
struct Bytes {
    pointer: NonNull<u8>,
    reservation: Reservation,
}

impl Bytes {
    fn zeroed(budget: &Budget, length: usize) -> Result<Self, AllocationError> {
        let layout = Layout::array::<u8>(length).map_err(|_| AllocationError::Capacity)?;
        let reservation = budget.reserve(length).map_err(|_| AllocationError::Limit)?;
        let pointer = if length == 0 {
            NonNull::dangling()
        } else {
            // SAFETY: layout has nonzero size and was validated above. A null
            // allocation is an ordinary error; the local reservation then drops.
            NonNull::new(unsafe { alloc_zeroed(layout) }).ok_or(AllocationError::Allocator)?
        };
        Ok(Self {
            pointer,
            reservation,
        })
    }

    fn from_slice(budget: &Budget, input: &[u8]) -> Result<Self, AllocationError> {
        let mut bytes = Self::zeroed(budget, input.len())?;
        bytes.as_mut_slice().copy_from_slice(input);
        Ok(bytes)
    }

    fn as_slice(&self) -> &[u8] {
        // SAFETY: the allocation contains exactly `bytes` initialized bytes.
        // Empty storage uses a non-null aligned dangling pointer. The slice
        // cannot outlive self and mutation requires an exclusive borrow.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.reservation.bytes) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: same storage invariant as as_slice, with exclusive ownership.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.reservation.bytes) }
    }

    fn try_clone(&self) -> Result<Self, AllocationError> {
        Self::from_slice(&Budget(self.reservation.ledger.clone()), self.as_slice())
    }

    fn append(&mut self, suffix: &[u8]) -> Result<(), AllocationError> {
        if suffix.is_empty() {
            return Ok(());
        }
        let length = self
            .reservation
            .bytes
            .checked_add(suffix.len())
            .ok_or(AllocationError::Capacity)?;
        // Both buffers exist during copying, so reserve the full new buffer.
        let mut replacement = Self::zeroed(&Budget(self.reservation.ledger.clone()), length)?;
        let (prefix, tail) = replacement
            .as_mut_slice()
            .split_at_mut(self.reservation.bytes);
        prefix.copy_from_slice(self.as_slice());
        tail.copy_from_slice(suffix);
        *self = replacement;
        Ok(())
    }
}

impl Drop for Bytes {
    fn drop(&mut self) {
        if self.reservation.bytes != 0 {
            let layout =
                Layout::array::<u8>(self.reservation.bytes).expect("validated at allocation");
            // SAFETY: pointer came from alloc_zeroed with this exact layout,
            // is uniquely owned, and has not been freed. Drop releases the
            // reservation field only after this allocation is deallocated.
            unsafe { dealloc(self.pointer.as_ptr(), layout) };
        }
    }
}

#[test]
fn byte_growth_accounts_for_copy_peak_and_preserves_original_on_failure() {
    let budget = Budget::new(9);
    let mut bytes = Bytes::from_slice(&budget, b"abcd").unwrap();
    assert_eq!(bytes.append(b"ef"), Err(AllocationError::Limit));
    assert_eq!(bytes.as_slice(), b"abcd");
    assert_eq!(budget.0.live.get(), 4);
    bytes.append(b"e").unwrap();
    assert_eq!(bytes.as_slice(), b"abcde");
    assert_eq!(budget.0.live.get(), 5);
    assert_eq!(budget.0.peak.get(), 9);
    assert!(bytes.try_clone().is_err());
    bytes.append(b"").unwrap();
    drop(bytes);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn byte_copies_reserve_separately_and_empty_storage_needs_no_allocation() {
    let budget = Budget::new(8);
    let bytes = Bytes::from_slice(&budget, b"data").unwrap();
    let mut copy = bytes.try_clone().unwrap();
    copy.as_mut_slice()[0] = b'D';
    assert_eq!(bytes.as_slice(), b"data");
    assert_eq!(copy.as_slice(), b"Data");
    assert_eq!(budget.0.live.get(), 8);
    drop((bytes, copy));
    assert_eq!(budget.0.live.get(), 0);
    let empty = Bytes::from_slice(&Budget::new(0), b"").unwrap();
    assert_eq!(empty.as_slice(), b"");
    assert!(empty.try_clone().is_ok());
    assert!(matches!(
        Bytes::zeroed(&budget, usize::MAX),
        Err(AllocationError::Capacity)
    ));
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn aggregate_limit_is_atomic_and_released_capacity_can_be_reused() {
    let budget = Budget::new(100);
    let mut first = budget.reserve(60).unwrap();
    let second = budget.clone().reserve(40).unwrap();
    assert!(budget.reserve(1).is_err());
    assert_eq!(first.resize(61), Err(LimitExceeded));
    assert_eq!(first.bytes, 60);
    assert_eq!(budget.0.live.get(), 100);
    drop(second);
    first.resize(100).unwrap();
    first.resize(20).unwrap();
    let third = budget.reserve(80).unwrap();
    assert_eq!(budget.0.peak.get(), 100);
    drop((first, third));
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn arithmetic_overflow_does_not_corrupt_reservations() {
    let budget = Budget::new(usize::MAX);
    let first = budget.reserve(usize::MAX - 1).unwrap();
    assert!(budget.reserve(2).is_err());
    assert_eq!(budget.0.live.get(), usize::MAX - 1);
    let second = budget.reserve(1).unwrap();
    drop((first, second));
    assert_eq!(budget.0.live.get(), 0);
    let zero = Budget::new(0);
    assert!(zero.reserve(0).is_ok());
    assert!(zero.reserve(1).is_err());
}

#[test]
fn shared_allocation_outlives_budget_handle_until_its_last_owner_drops() {
    let budget = Budget::new(64);
    let ledger = Rc::downgrade(&budget.0);
    let allocation = Rc::new(budget.reserve(64).unwrap());
    let alias = allocation.clone();
    drop(budget);
    drop(allocation);
    assert_eq!(ledger.upgrade().unwrap().live.get(), 64);
    drop(alias);
    assert!(ledger.upgrade().is_none());
}
