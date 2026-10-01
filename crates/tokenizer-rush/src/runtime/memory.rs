//! Reservation primitive for the pending managed-value storage migration.
//! Counts requested bytes, not allocator overhead or resident memory.
use std::alloc::{Layout, alloc, alloc_zeroed, dealloc};
use std::ptr::NonNull;
use std::{cell::Cell, rc::Rc};

#[cfg(test)]
thread_local! {
    static FAIL_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
}

fn allocate(layout: Layout, zeroed: bool) -> *mut u8 {
    if layout.size() == 0 {
        return std::ptr::null_mut();
    }
    #[cfg(test)]
    if FAIL_AFTER.with(|remaining| match remaining.get() {
        Some(0) => true,
        Some(n) => {
            remaining.set(Some(n - 1));
            false
        }
        None => false,
    }) {
        return std::ptr::null_mut();
    }
    // SAFETY: Layout is valid by construction and zero-sized requests were
    // rejected. Callers own a successful allocation and handle null on failure.
    unsafe {
        if zeroed {
            alloc_zeroed(layout)
        } else {
            alloc(layout)
        }
    }
}

#[cfg(test)]
struct AllocationFailure(Option<usize>);
#[cfg(test)]
impl AllocationFailure {
    fn after(successes: usize) -> Self {
        Self(FAIL_AFTER.with(|remaining| remaining.replace(Some(successes))))
    }
}
#[cfg(test)]
impl Drop for AllocationFailure {
    fn drop(&mut self) {
        FAIL_AFTER.with(|remaining| remaining.set(self.0));
    }
}

#[derive(Debug)]
struct Ledger {
    limit: usize,
    live: Cell<usize>,
    peak: Cell<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct Budget(Rc<Ledger>);

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
    pub(super) fn new(limit: usize) -> Self {
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
pub(super) enum AllocationError {
    Limit,
    Capacity,
    Allocator,
    #[cfg(test)]
    Depth,
    #[cfg(test)]
    Unsupported,
    #[cfg(test)]
    Steps,
    #[cfg(test)]
    Cancelled,
}

#[cfg(test)]
struct Transfer<'a> {
    remaining: usize,
    cancellation: &'a super::CancellationToken,
}
#[cfg(test)]
impl Transfer<'_> {
    fn charge(&mut self, amount: usize) -> Result<(), AllocationError> {
        if self.cancellation.is_cancelled() {
            return Err(AllocationError::Cancelled);
        }
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or(AllocationError::Steps)?;
        Ok(())
    }
}

/// Data-only boundary prototype. Executable values still need managed storage
/// before this can replace the runtime's Value.
#[derive(Debug)]
#[cfg(test)]
enum Data {
    Number(f64),
    Angle(f64),
    Bool(bool),
    Null,
    String(Bytes),
    Vector(Slots<f64>),
    List(Slots<Data>),
    Tuple(Slots<Data>),
    Variant(&'static str, Slots<Data>),
    Record(Slots<(Bytes, Data)>),
    Matrix(Slots<crate::Matrix4>),
    Quaternion(Slots<crate::Quaternion>),
    Polygon(Slots<[f64; 2]>),
    Mesh {
        vertices: Slots<[f64; 3]>,
        triangles: Slots<[usize; 3]>,
    },
}

#[cfg(test)]
impl Data {
    fn import(
        value: &super::Value<'_>,
        budget: &Budget,
        depth: usize,
    ) -> Result<Self, AllocationError> {
        Self::import_checked(
            value,
            budget,
            depth,
            &mut Transfer {
                remaining: usize::MAX,
                cancellation: &super::CancellationToken::default(),
            },
        )
    }

    fn import_checked(
        value: &super::Value<'_>,
        budget: &Budget,
        depth: usize,
        transfer: &mut Transfer<'_>,
    ) -> Result<Self, AllocationError> {
        use super::Value;
        transfer.charge(1)?;
        if depth > 64 {
            return Err(AllocationError::Depth);
        }
        Ok(match value {
            Value::Number(n) => Self::Number(*n),
            Value::Angle(n) => Self::Angle(*n),
            Value::Bool(b) => Self::Bool(*b),
            Value::Null => Self::Null,
            Value::String(s) => Self::String(Bytes::copy_checked(budget, s.as_bytes(), transfer)?),
            Value::Matrix(matrix) => {
                let mut items = Slots::new(budget, 1)?;
                items.push((**matrix).clone())?;
                Self::Matrix(items)
            }
            Value::Quaternion(quaternion) => {
                let mut items = Slots::new(budget, 1)?;
                items.push((**quaternion).clone())?;
                Self::Quaternion(items)
            }
            Value::Polygon(polygon) => {
                Self::Polygon(Slots::copy_from_slice(budget, polygon.points(), transfer)?)
            }
            Value::Mesh(mesh) => Self::Mesh {
                vertices: Slots::copy_from_slice(budget, mesh.vertices(), transfer)?,
                triangles: Slots::copy_from_slice(budget, mesh.triangles(), transfer)?,
            },
            Value::Vector(v) => {
                let mut items = Slots::new(budget, v.len())?;
                for item in v {
                    transfer.charge(1)?;
                    items.push(*item)?;
                }
                Self::Vector(items)
            }
            Value::List(v) | Value::Tuple(v) | Value::Variant(_, v) => {
                let mut items = Slots::new(budget, v.len())?;
                for item in v {
                    items.push(Self::import_checked(item, budget, depth + 1, transfer)?)?;
                }
                match value {
                    Value::List(_) => Self::List(items),
                    Value::Tuple(_) => Self::Tuple(items),
                    Value::Variant(tag, _) => Self::Variant(tag, items),
                    _ => unreachable!(),
                }
            }
            Value::Record(v) => {
                let mut fields = Slots::new(budget, v.len())?;
                for (key, value) in v {
                    let key = Bytes::copy_checked(budget, key.as_bytes(), transfer)?;
                    fields.push((
                        key,
                        Self::import_checked(value, budget, depth + 1, transfer)?,
                    ))?;
                }
                Self::Record(fields)
            }
            _ => return Err(AllocationError::Unsupported),
        })
    }

    fn try_copy(
        &self,
        budget: &Budget,
        transfer: &mut Transfer<'_>,
    ) -> Result<Self, AllocationError> {
        transfer.charge(1)?;
        Ok(match self {
            Self::Number(n) => Self::Number(*n),
            Self::Angle(n) => Self::Angle(*n),
            Self::Bool(b) => Self::Bool(*b),
            Self::Null => Self::Null,
            Self::String(s) => Self::String(Bytes::copy_checked(budget, s.as_slice(), transfer)?),
            Self::Vector(v) => {
                Self::Vector(Slots::copy_from_slice(budget, v.as_slice(), transfer)?)
            }
            Self::Polygon(v) => {
                Self::Polygon(Slots::copy_from_slice(budget, v.as_slice(), transfer)?)
            }
            Self::Mesh {
                vertices,
                triangles,
            } => Self::Mesh {
                vertices: Slots::copy_from_slice(budget, vertices.as_slice(), transfer)?,
                triangles: Slots::copy_from_slice(budget, triangles.as_slice(), transfer)?,
            },
            Self::Matrix(v) => {
                let mut copy = Slots::new(budget, 1)?;
                copy.push(v.as_slice()[0].clone())?;
                Self::Matrix(copy)
            }
            Self::Quaternion(v) => {
                let mut copy = Slots::new(budget, 1)?;
                copy.push(v.as_slice()[0].clone())?;
                Self::Quaternion(copy)
            }
            Self::List(v) | Self::Tuple(v) | Self::Variant(_, v) => {
                let mut copy = Slots::new(budget, v.length)?;
                for item in v.as_slice() {
                    copy.push(item.try_copy(budget, transfer)?)?;
                }
                match self {
                    Self::List(_) => Self::List(copy),
                    Self::Tuple(_) => Self::Tuple(copy),
                    Self::Variant(tag, _) => Self::Variant(tag, copy),
                    _ => unreachable!(),
                }
            }
            Self::Record(v) => {
                let mut copy = Slots::new(budget, v.length)?;
                for (key, value) in v.as_slice() {
                    let key = Bytes::copy_checked(budget, key.as_slice(), transfer)?;
                    copy.push((key, value.try_copy(budget, transfer)?))?;
                }
                Self::Record(copy)
            }
        })
    }

    fn matches(&self, value: &super::Value<'_>) -> bool {
        use super::Value;
        fn items_match(items: &Slots<Data>, values: &[Value<'_>]) -> bool {
            items.length == values.len()
                && items
                    .as_slice()
                    .iter()
                    .zip(values)
                    .all(|(a, b)| a.matches(b))
        }
        match (self, value) {
            (Self::Number(a), Value::Number(b)) | (Self::Angle(a), Value::Angle(b)) => {
                a.to_bits() == b.to_bits()
            }
            (Self::Bool(a), Value::Bool(b)) => a == b,
            (Self::Null, Value::Null) => true,
            (Self::String(a), Value::String(b)) => a.as_slice() == b.as_bytes(),
            (Self::Vector(a), Value::Vector(b)) => a.as_slice() == b,
            (Self::Matrix(a), Value::Matrix(b)) => a.as_slice() == std::slice::from_ref(b.as_ref()),
            (Self::Quaternion(a), Value::Quaternion(b)) => {
                a.as_slice() == std::slice::from_ref(b.as_ref())
            }
            (Self::Polygon(a), Value::Polygon(b)) => a.as_slice() == b.points(),
            (
                Self::Mesh {
                    vertices,
                    triangles,
                },
                Value::Mesh(b),
            ) => vertices.as_slice() == b.vertices() && triangles.as_slice() == b.triangles(),
            (Self::List(a), Value::List(b)) | (Self::Tuple(a), Value::Tuple(b)) => {
                items_match(a, b)
            }
            (Self::Variant(a, items), Value::Variant(b, values)) => {
                a == b && items_match(items, values)
            }
            (Self::Record(a), Value::Record(b)) => {
                a.length == b.len()
                    && a.as_slice()
                        .iter()
                        .zip(b)
                        .all(|((key, item), (name, value))| {
                            key.as_slice() == name.as_bytes() && item.matches(value)
                        })
            }
            _ => false,
        }
    }
}

#[test]
fn managed_data_imports_runtime_results_and_releases_all_nested_storage() {
    let value = super::evaluate(
        "{name:'привет',values:[vec3(1,2,3),Some(-0),degrees(90)],pair:(true,null)}",
        1000,
    )
    .unwrap();
    let budget = Budget::new(100_000);
    let data = Data::import(&value, &budget, 0).unwrap();
    assert!(data.matches(&value));
    let live = budget.0.live.get();
    assert!(live > 0);
    let tight = Budget::new(live - 1);
    assert!(matches!(
        Data::import(&value, &tight, 0),
        Err(AllocationError::Limit)
    ));
    assert_eq!(
        tight.0.live.get(),
        0,
        "Partial record construction leaked reservations"
    );
    let exact = Budget::new(live);
    let imported = Data::import(&value, &exact, 0).unwrap();
    assert!(imported.matches(&value));
    assert!(matches!(
        Data::import(&super::Value::String("x".into()), &exact, 0),
        Err(AllocationError::Limit)
    ));
    drop((data, imported));
    assert_eq!(budget.0.live.get(), 0);
    assert_eq!(exact.0.live.get(), 0);
}

#[test]
fn unsupported_and_deep_data_fail_without_leaving_allocations() {
    let budget = Budget::new(1_000_000);
    let unsupported = super::evaluate("['prefix', x=>x]", 1000).unwrap();
    assert!(matches!(
        Data::import(&unsupported, &budget, 0),
        Err(AllocationError::Unsupported)
    ));
    assert_eq!(budget.0.live.get(), 0);
    let mut deep = super::Value::Null;
    for _ in 0..66 {
        deep = super::Value::List(vec![deep]);
    }
    assert!(matches!(
        Data::import(&deep, &budget, 0),
        Err(AllocationError::Depth)
    ));
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn geometry_uses_the_same_budget_and_mesh_failure_releases_vertices() {
    for (source, expected) in [
        ("identity()", std::mem::size_of::<crate::Matrix4>()),
        (
            "axis_angle(vec3(0,1,0),degrees(90))",
            std::mem::size_of::<crate::Quaternion>(),
        ),
        (
            "polygon([vec2(0,0),vec2(1,0),vec2(0,1)])",
            3 * std::mem::size_of::<[f64; 2]>(),
        ),
        (
            "mesh([vec3(0,0,0),vec3(1,0,0),vec3(0,1,0)],[[0,1,2]])",
            3 * std::mem::size_of::<[f64; 3]>() + std::mem::size_of::<[usize; 3]>(),
        ),
    ] {
        let value = super::evaluate(source, 10_000).unwrap();
        let budget = Budget::new(expected);
        let data = Data::import(&value, &budget, 0).unwrap();
        assert!(data.matches(&value), "{source}");
        assert_eq!(budget.0.live.get(), expected, "{source}");
        drop(data);
        assert_eq!(budget.0.live.get(), 0);
        let tight = Budget::new(expected - 1);
        assert!(
            matches!(Data::import(&value, &tight, 0), Err(AllocationError::Limit)),
            "{source}"
        );
        assert_eq!(tight.0.live.get(), 0, "{source}");
    }
}

#[test]
fn transfer_budget_and_cancellation_release_partial_data() {
    let value = super::Value::List(vec![
        super::Value::String("first".into()),
        super::Value::String("second".into()),
    ]);
    let budget = Budget::new(10_000);
    let token = super::CancellationToken::default();
    // Three value nodes and eleven bytes of text.
    for steps in 0..14 {
        let mut transfer = Transfer {
            remaining: steps,
            cancellation: &token,
        };
        assert!(matches!(
            Data::import_checked(&value, &budget, 0, &mut transfer),
            Err(AllocationError::Steps)
        ));
        assert_eq!(budget.0.live.get(), 0);
    }
    let mut transfer = Transfer {
        remaining: 14,
        cancellation: &token,
    };
    let data = Data::import_checked(&value, &budget, 0, &mut transfer).unwrap();
    assert!(data.matches(&value));
    assert_eq!(transfer.remaining, 0);
    drop(data);
    token.cancel();
    let mut transfer = Transfer {
        remaining: 100,
        cancellation: &token,
    };
    assert!(matches!(
        Data::import_checked(&value, &budget, 0, &mut transfer),
        Err(AllocationError::Cancelled)
    ));
    assert_eq!(transfer.remaining, 100);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn managed_copy_preserves_source_on_failure_and_charges_destination_budget() {
    let value = super::evaluate("{items:[Some('abc'),vec3(1,2,3),identity(),axis_angle(vec3(1,0,0),0)],pair:(true,null),angle:degrees(90),number:-0,shape:polygon([vec2(0,0),vec2(1,0),vec2(0,1)]),model:mesh([vec3(0,0,0),vec3(1,0,0),vec3(0,1,0)],[[0,1,2]])}", 10_000).unwrap();
    let source_budget = Budget::new(100_000);
    let source = Data::import(&value, &source_budget, 0).unwrap();
    let bytes = source_budget.0.live.get();
    let token = super::CancellationToken::default();
    for limit in [0, bytes / 2, bytes - 1] {
        let destination = Budget::new(limit);
        let mut transfer = Transfer {
            remaining: 100_000,
            cancellation: &token,
        };
        assert!(matches!(
            source.try_copy(&destination, &mut transfer),
            Err(AllocationError::Limit)
        ));
        assert_eq!(destination.0.live.get(), 0);
        assert!(source.matches(&value));
        assert_eq!(source_budget.0.live.get(), bytes);
    }
    let destination = Budget::new(bytes);
    let mut transfer = Transfer {
        remaining: 100_000,
        cancellation: &token,
    };
    let copy = source.try_copy(&destination, &mut transfer).unwrap();
    assert_eq!(destination.0.live.get(), bytes);
    drop(source);
    assert_eq!(source_budget.0.live.get(), 0);
    assert!(copy.matches(&value));
    drop(copy);
    assert_eq!(destination.0.live.get(), 0);
}

/// Single-threaded shared ownership with one reservation for the whole node.
/// No weak pointers: environments only expose strong, immutable ownership.
struct SharedNode<T> {
    references: Cell<usize>,
    value: T,
    reservation: std::mem::ManuallyDrop<Reservation>,
}
pub(super) struct Shared<T> {
    pointer: NonNull<SharedNode<T>>,
    // Match Rc ownership/drop checking and prohibit Send/Sync even for T: Send.
    marker: std::marker::PhantomData<Rc<T>>,
}
impl<T> Shared<T> {
    pub(super) fn new(budget: &Budget, value: T) -> Result<Self, AllocationError> {
        let layout = Layout::new::<SharedNode<T>>();
        let reservation = budget
            .reserve(layout.size())
            .map_err(|_| AllocationError::Limit)?;
        let pointer = NonNull::new(allocate(layout, false).cast::<SharedNode<T>>())
            .ok_or(AllocationError::Allocator)?;
        // SAFETY: freshly allocated, correctly aligned storage for one node.
        unsafe {
            pointer.as_ptr().write(SharedNode {
                references: Cell::new(1),
                value,
                reservation: std::mem::ManuallyDrop::new(reservation),
            })
        };
        Ok(Self {
            pointer,
            marker: std::marker::PhantomData,
        })
    }
    pub(super) fn as_ptr(this: &Self) -> *const T {
        // SAFETY: every handle owns a strong reference to a live node.
        unsafe { std::ptr::addr_of!((*this.pointer.as_ptr()).value) }
    }
    pub(super) fn strong_count(this: &Self) -> usize {
        // SAFETY: same live-node invariant as as_ptr; access is single-threaded.
        unsafe { this.pointer.as_ref().references.get() }
    }
}
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        let count = Self::strong_count(self)
            .checked_add(1)
            .expect("shared reference count overflow");
        // SAFETY: the node is live; Cell permits mutation without a mutable T.
        unsafe { self.pointer.as_ref().references.set(count) };
        Self {
            pointer: self.pointer,
            marker: std::marker::PhantomData,
        }
    }
}
impl<T> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: a handle keeps value initialized and alive for this borrow.
        unsafe { &*Self::as_ptr(self) }
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(formatter)
    }
}
impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl<T> Drop for Shared<T> {
    fn drop(&mut self) {
        let count = Self::strong_count(self);
        if count > 1 {
            // SAFETY: another handle keeps the node alive after this decrement.
            unsafe { self.pointer.as_ref().references.set(count - 1) };
            return;
        }
        // SAFETY: this is the last owner. Move the reservation into a storage
        // guard before dropping T so memory is freed even if its Drop unwinds.
        // No reference to the node is used after the guard deallocates it.
        unsafe {
            let pointer = self.pointer.as_ptr();
            let _storage = Storage {
                pointer: self.pointer,
                layout: Layout::new::<SharedNode<T>>(),
                reservation: std::mem::ManuallyDrop::take(&mut (*pointer).reservation),
            };
            std::ptr::drop_in_place(std::ptr::addr_of_mut!((*pointer).value));
        }
    }
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
pub(super) struct Slots<T> {
    storage: Storage<T>,
    capacity: usize,
    length: usize,
}

impl<T> Slots<T> {
    pub(super) fn capacity(&self) -> usize {
        self.capacity
    }
    #[cfg(test)]
    fn copy_from_slice(
        budget: &Budget,
        values: &[T],
        transfer: &mut Transfer<'_>,
    ) -> Result<Self, AllocationError>
    where
        T: Copy,
    {
        transfer.charge(values.len())?;
        let mut result = Self::new(budget, values.len())?;
        for value in values {
            transfer.charge(0)?;
            result.push(*value)?;
        }
        Ok(result)
    }
    pub(super) fn new(budget: &Budget, capacity: usize) -> Result<Self, AllocationError> {
        let layout = Layout::array::<T>(capacity).map_err(|_| AllocationError::Capacity)?;
        let reservation = budget
            .reserve(layout.size())
            .map_err(|_| AllocationError::Limit)?;
        let pointer = if layout.size() == 0 {
            NonNull::dangling()
        } else {
            // No element is read until initialized.
            NonNull::new(allocate(layout, false).cast::<T>()).ok_or(AllocationError::Allocator)?
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

    pub(super) fn reserve(&mut self, additional: usize) -> Result<(), AllocationError> {
        let needed = self
            .length
            .checked_add(additional)
            .ok_or(AllocationError::Capacity)?;
        if needed > self.capacity {
            self.grow(self.capacity.checked_mul(2).unwrap_or(needed).max(needed))?;
        }
        Ok(())
    }

    pub(super) fn push(&mut self, value: T) -> Result<(), AllocationError> {
        self.reserve(1)?;
        // SAFETY: this slot lies in spare capacity and is currently uninitialized.
        unsafe { self.storage.pointer.as_ptr().add(self.length).write(value) };
        self.length += 1;
        Ok(())
    }

    pub(super) fn pop(&mut self) -> Option<T> {
        if self.length == 0 {
            return None;
        }
        self.length -= 1;
        // SAFETY: the previous last slot was initialized. Reducing length
        // transfers its ownership to the caller and excludes it from Drop.
        Some(unsafe { self.storage.pointer.as_ptr().add(self.length).read() })
    }
}

impl<T> std::ops::Deref for Slots<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> std::ops::DerefMut for Slots<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: only the initialized prefix is exposed, under an exclusive
        // borrow that prevents growth, deallocation or simultaneous access.
        unsafe { std::slice::from_raw_parts_mut(self.storage.pointer.as_ptr(), self.length) }
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
#[cfg(test)]
struct Bytes {
    pointer: NonNull<u8>,
    reservation: Reservation,
}

#[cfg(test)]
impl Bytes {
    fn copy_checked(
        budget: &Budget,
        input: &[u8],
        transfer: &mut Transfer<'_>,
    ) -> Result<Self, AllocationError> {
        // Reject an unaffordable copy before allocating the destination.
        transfer.charge(input.len())?;
        let mut result = Self::zeroed(budget, input.len())?;
        for (target, source) in result
            .as_mut_slice()
            .chunks_mut(4096)
            .zip(input.chunks(4096))
        {
            transfer.charge(0)?;
            target.copy_from_slice(source);
        }
        Ok(result)
    }
    fn zeroed(budget: &Budget, length: usize) -> Result<Self, AllocationError> {
        let layout = Layout::array::<u8>(length).map_err(|_| AllocationError::Capacity)?;
        let reservation = budget.reserve(length).map_err(|_| AllocationError::Limit)?;
        let pointer = if length == 0 {
            NonNull::dangling()
        } else {
            // Layout has nonzero size and was validated above. A null
            // allocation is an ordinary error; the local reservation then drops.
            NonNull::new(allocate(layout, true)).ok_or(AllocationError::Allocator)?
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

#[cfg(test)]
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
fn allocator_failure_during_growth_preserves_storage_and_reservation() {
    let budget = Budget::new(10_000);
    let mut bytes = Bytes::from_slice(&budget, b"original").unwrap();
    let mut slots = Slots::new(&budget, 1).unwrap();
    slots.push(42u64).unwrap();
    let before = budget.0.live.get();
    {
        let _failure = AllocationFailure::after(0);
        assert_eq!(bytes.append(b" appended"), Err(AllocationError::Allocator));
        assert_eq!(slots.push(43), Err(AllocationError::Allocator));
    }
    assert_eq!(bytes.as_slice(), b"original");
    assert_eq!(slots.as_slice(), &[42]);
    assert_eq!(budget.0.live.get(), before);
    bytes.append(b" appended").unwrap();
    slots.push(43).unwrap();
    drop((bytes, slots));
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn runtime_cell_allocation_failure_is_recoverable_between_calls() {
    let program = super::Program::compile("mut count=0; fn grow() { mut local=1; return local }; fn next() { count+=1; return count }").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(1000);
    for index in 0..4 {
        let mut instance = program
            .instantiate(limits, &token, &[], &[], &[], &[])
            .unwrap();
        {
            let _failure = AllocationFailure::after(index + 1);
            let error = instance.call("grow", &[], limits).unwrap_err();
            assert_eq!(error.message, "Runtime cell allocation failed: Allocator");
        }
        assert_eq!(instance.runtime.cells.len(), 1);
        assert_eq!(instance.runtime.cell_ids.len(), 1);
        assert!(Rc::ptr_eq(
            &instance.runtime.cells.storage.reservation.ledger,
            &instance.runtime.cell_ids.storage.reservation.ledger
        ));
        assert_eq!(instance.get("count"), Some(super::Value::Number(0.0)));
        assert_eq!(
            instance.call("next", &[], limits).unwrap(),
            super::Value::Number(1.0)
        );
        assert_eq!(
            instance.call("grow", &[], limits).unwrap(),
            super::Value::Number(1.0)
        );
        // Reusing and releasing an existing slot must not allocate, even when
        // the allocator would refuse every request.
        let _failure = AllocationFailure::after(0);
        let cell = instance
            .runtime
            .allocate_cell(super::Value::Number(1.0), None, instance.span)
            .unwrap();
        drop(cell);
        instance.runtime.reclaim_cells();
        drop(instance);
    }
}

#[test]
fn popping_transfers_ownership_without_releasing_the_element_early() {
    let budget = Budget::new(1000);
    let mut slots = Slots::new(&budget, 1).unwrap();
    slots
        .push(Bytes::from_slice(&budget, b"owned").unwrap())
        .unwrap();
    let value = slots.pop().unwrap();
    assert!(slots.pop().is_none());
    drop(slots);
    assert_eq!(budget.0.live.get(), 5);
    assert_eq!(value.as_slice(), b"owned");
    drop(value);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn every_failed_nested_allocation_is_released_and_retry_succeeds() {
    let value = super::evaluate(
        "{name:'scene',values:[Some('text'),vec3(1,2,3),identity()],pair:(true,null)}",
        1000,
    )
    .unwrap();
    let budget = Budget::new(100_000);
    let mut completed = false;
    let mut failures = 0;
    for index in 0..64 {
        let failure = AllocationFailure::after(index);
        match Data::import(&value, &budget, 0) {
            Err(AllocationError::Allocator) => {
                failures += 1;
            }
            Ok(data) => {
                assert!(data.matches(&value));
                completed = true;
            }
            result => panic!("Unexpected result at allocation {index}: {result:?}"),
        }
        drop(failure);
        assert_eq!(budget.0.live.get(), 0, "allocation {index}");
        if completed {
            break;
        }
    }
    assert!(completed && failures > 5);
    let retry = Data::import(&value, &budget, 0).unwrap();
    assert!(retry.matches(&value));
    drop(retry);
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

#[test]
fn module_stack_allocation_failure_leaves_instance_reusable() {
    let program = super::Program::compile("fn load() { import data; return data.answer }").unwrap();
    let module = super::Program::compile("{answer: 42}").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(1000);
    let mut instance = program
        .instantiate(limits, &token, &[], &[], &[], &[("data", &module)])
        .unwrap();
    {
        let _failure = AllocationFailure::after(1);
        let error = instance.call("load", &[], limits).unwrap_err();
        assert_eq!(
            error.message,
            "Runtime module stack allocation failed: Allocator"
        );
    }
    assert!(instance.runtime.loading.is_empty());
    assert!(instance.runtime.module_cache.is_empty());
    assert_eq!(instance.runtime.module, None);
    assert_eq!(instance.runtime.depth, 0);
    assert!(Rc::ptr_eq(
        &instance.runtime.loading.storage.reservation.ledger,
        &instance.runtime.cells.storage.reservation.ledger
    ));
    assert_eq!(
        instance.call("load", &[], limits).unwrap(),
        super::Value::Number(42.0)
    );
}

#[test]
fn module_cache_failure_preserves_previous_exports_and_allows_retry() {
    let program = super::Program::compile(
        "fn first() { import z; return z.answer }; fn second() { import a; return a.answer }",
    )
    .unwrap();
    let module = super::Program::compile("{answer: 42}").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(1000);
    let mut instance = program
        .instantiate(
            limits,
            &token,
            &[],
            &[],
            &[],
            &[("z", &module), ("a", &module)],
        )
        .unwrap();
    assert_eq!(
        instance.call("first", &[], limits).unwrap(),
        super::Value::Number(42.0)
    );
    {
        let _failure = AllocationFailure::after(1);
        let error = instance.call("second", &[], limits).unwrap_err();
        assert_eq!(
            error.message,
            "Runtime module cache allocation failed: Allocator"
        );
    }
    assert!(instance.runtime.loading.is_empty());
    assert_eq!(instance.runtime.module_cache.len(), 1);
    assert_eq!(instance.runtime.module, None);
    assert_eq!(instance.runtime.depth, 0);
    assert_eq!(
        instance.call("second", &[], limits).unwrap(),
        super::Value::Number(42.0)
    );
    assert_eq!(instance.runtime.module_cache[0].0, "a");
    assert_eq!(instance.runtime.module_cache[1].0, "z");
}

#[test]
fn module_cache_is_reserved_before_body_and_for_nested_ancestors() {
    let program =
        super::Program::compile("fn load() { import outer; return outer.answer }").unwrap();
    let calls = Rc::new(Cell::new(0));
    let counter = calls.clone();
    let host = super::HostRegistration::new(
        "initialized",
        vec![],
        super::ValueType::Number,
        move |_, _| {
            counter.set(counter.get() + 1);
            Ok(super::Value::Number(0.0))
        },
    );
    let outer =
        super::Program::compile("initialized(); import inner; {answer: inner.answer}").unwrap();
    let inner = super::Program::compile("{answer: 42}").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(1000);
    let mut instance = program
        .instantiate(
            limits,
            &token,
            &[],
            &[],
            std::slice::from_ref(&host),
            &[("outer", &outer), ("inner", &inner)],
        )
        .unwrap();
    {
        // Allow the call environment and stack, then refuse the export reservation.
        let _failure = AllocationFailure::after(2);
        assert_eq!(
            instance.call("load", &[], limits).unwrap_err().message,
            "Runtime module cache allocation failed: Allocator"
        );
    }
    assert_eq!(calls.get(), 0);
    assert!(instance.runtime.loading.is_empty());
    assert!(instance.runtime.module_cache.is_empty());
    assert_eq!(
        instance.call("load", &[], limits).unwrap(),
        super::Value::Number(42.0)
    );
    assert_eq!(instance.runtime.module_cache.len(), 2);
    assert_eq!(calls.get(), 1);
    assert_eq!(
        instance.call("load", &[], limits).unwrap(),
        super::Value::Number(42.0)
    );
}

#[test]
fn environment_copy_and_extension_fail_without_changing_bindings() {
    let budget = Budget::new(100_000);
    let mut environment = super::Environment::new(&budget);
    environment.insert("z", super::Value::Number(1.0)).unwrap();
    let mut extra = super::Environment::new(&budget);
    extra.insert("a", super::Value::Number(2.0)).unwrap();
    let baseline = budget.0.live.get();
    {
        let _failure = AllocationFailure::after(0);
        assert!(matches!(
            environment.try_clone(),
            Err(AllocationError::Allocator)
        ));
        assert_eq!(budget.0.live.get(), baseline);
        assert_eq!(environment.extend(extra), Err(AllocationError::Allocator));
        assert!(!environment.contains_key("a"));
        assert!(environment.contains_key("z"));
        // Replacement of an existing binding needs no allocation.
        environment.insert("z", super::Value::Number(3.0)).unwrap();
    }
    let copy = environment.try_clone().unwrap();
    assert_eq!(copy, environment);
    assert!(Rc::ptr_eq(&copy.budget.0, &environment.budget.0));
    drop((copy, environment));
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn every_environment_allocation_failure_preserves_instance_and_releases_storage() {
    let program = super::Program::compile("let base=7; fn work(x) { let (a,b)=(x,base); if true { let f=v => v+a+b; return f(1) }; return 0 }").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(1000);
    let mut completed = false;
    let mut failures = 0;
    for index in 0..64 {
        let mut instance = program
            .instantiate(limits, &token, &[], &[], &[], &[])
            .unwrap();
        let ledger = instance.runtime.memory.0.clone();
        let failure = AllocationFailure::after(index);
        let result = instance.call("work", &[super::Value::Number(2.0)], limits);
        drop(failure);
        match result {
            Ok(value) => {
                assert_eq!(value, super::Value::Number(10.0));
                completed = true;
            }
            Err(error) => {
                assert_eq!(
                    error.message,
                    "Runtime environment allocation failed: Allocator"
                );
                failures += 1;
            }
        }
        assert_eq!(instance.runtime.depth, 0);
        assert_eq!(instance.runtime.module, None);
        assert_eq!(instance.get("base"), Some(super::Value::Number(7.0)));
        assert_eq!(
            instance
                .call("work", &[super::Value::Number(2.0)], limits)
                .unwrap(),
            super::Value::Number(10.0)
        );
        drop(instance);
        assert_eq!(ledger.live.get(), 0, "allocation {index}");
        if completed {
            break;
        }
    }
    assert!(completed && failures >= 4);
}

#[test]
fn environment_budget_counts_shared_parents_and_snapshot_copy_separately() {
    let slot = std::mem::size_of::<(&str, super::Binding<'_>)>();
    let node = std::mem::size_of::<SharedNode<super::Environment<'_>>>();
    let budget = Budget::new(slot * 2 + node);
    let mut parent = super::Environment::new(&budget);
    parent.insert("x", super::Value::Number(1.0)).unwrap();
    let parent = Shared::new(&budget, parent).unwrap();
    let child = super::Environment::child(parent.clone());
    assert_eq!(budget.0.live.get(), slot + node);
    let copy = parent.try_clone().unwrap();
    assert_eq!(budget.0.live.get(), slot * 2 + node);
    assert!(matches!(parent.try_clone(), Err(AllocationError::Limit)));
    assert_eq!(budget.0.live.get(), slot * 2 + node);
    drop(copy);
    drop(parent);
    assert_eq!(budget.0.live.get(), slot + node);
    assert!(child.contains_key("x"));
    drop(child);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn environment_extension_only_reserves_new_names_and_is_atomic_at_limit() {
    let slot = std::mem::size_of::<(&str, super::Binding<'_>)>();
    let budget = Budget::new(slot * 2);
    let mut original = super::Environment::new(&budget);
    original.insert("x", super::Value::Number(1.0)).unwrap();
    let mut replacement = super::Environment::new(&budget);
    replacement.insert("x", super::Value::Number(2.0)).unwrap();
    // All capacity is occupied. Replacing an existing name still succeeds.
    original.extend(replacement).unwrap();
    assert_eq!(
        original.get("x"),
        Some(&super::Binding::Value(super::Value::Number(2.0)))
    );
    assert_eq!(budget.0.live.get(), slot);
    let mut extra = super::Environment::new(&budget);
    extra.insert("y", super::Value::Number(3.0)).unwrap();
    assert_eq!(original.extend(extra), Err(AllocationError::Limit));
    assert!(!original.contains_key("y"));
    assert_eq!(
        original.get("x"),
        Some(&super::Binding::Value(super::Value::Number(2.0)))
    );
    assert_eq!(budget.0.live.get(), slot);
    drop(original);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn returned_closure_keeps_capture_reservation_until_last_owner_drops() {
    let program = super::Program::compile("let base=7; x => base+x").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(1000);
    let instance = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let ledger = instance.runtime.memory.0.clone();
    let exported = instance.initial_value.clone();
    drop(instance);
    assert_eq!(
        ledger.live.get(),
        std::mem::size_of::<(&str, super::Binding<'_>)>()
            + std::mem::size_of::<SharedNode<super::Environment<'_>>>()
    );
    let shared = exported.clone();
    drop(exported);
    assert!(ledger.live.get() > 0);
    drop(shared);
    assert_eq!(ledger.live.get(), 0);
}

#[test]
fn shared_node_is_fallible_aligned_and_drops_payload_once() {
    #[derive(Debug)]
    #[repr(align(128))]
    struct Payload(Rc<Cell<usize>>);
    impl Drop for Payload {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let budget = Budget::new(10_000);
    {
        let _failure = AllocationFailure::after(0);
        assert!(matches!(
            Shared::new(&budget, Payload(drops.clone())),
            Err(AllocationError::Allocator)
        ));
    }
    assert_eq!(drops.get(), 1);
    assert_eq!(budget.0.live.get(), 0);
    let first = Shared::new(&budget, Payload(drops.clone())).unwrap();
    assert_eq!(Shared::as_ptr(&first) as usize % 128, 0);
    let second = first.clone();
    assert_eq!(Shared::strong_count(&first), 2);
    assert_eq!(Shared::as_ptr(&first), Shared::as_ptr(&second));
    assert_eq!(
        budget.0.live.get(),
        std::mem::size_of::<SharedNode<Payload>>()
    );
    drop(first);
    assert_eq!(drops.get(), 1);
    assert_eq!(Shared::strong_count(&second), 1);
    drop(second);
    assert_eq!(drops.get(), 2);
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn shared_node_releases_storage_when_payload_drop_panics() {
    struct Panics;
    impl Drop for Panics {
        fn drop(&mut self) {
            panic!("expected destructor failure");
        }
    }
    let budget = Budget::new(1000);
    let value = Shared::new(&budget, Panics).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value)));
    assert!(result.is_err());
    assert_eq!(budget.0.live.get(), 0);
}

#[test]
fn nested_import_allocation_failures_restore_context_and_release_captures() {
    let program =
        super::Program::compile("fn load() { import outer; return outer.answer() }").unwrap();
    let inner = super::Program::compile("let n=42; {answer: () => n}").unwrap();
    let outer = super::Program::compile("import inner; {answer: () => inner.answer()}").unwrap();
    let token = super::CancellationToken::default();
    let limits = super::ExecutionLimits::new(10_000);
    let mut completed = false;
    let mut failures = 0;
    for index in 0..128 {
        let mut instance = program
            .instantiate(
                limits,
                &token,
                &[],
                &[],
                &[],
                &[("outer", &outer), ("inner", &inner)],
            )
            .unwrap();
        let ledger = instance.runtime.memory.0.clone();
        let references = instance.runtime.current_references.clone();
        let failure = AllocationFailure::after(index);
        let result = instance.call("load", &[], limits);
        drop(failure);
        match result {
            Ok(value) => {
                assert_eq!(value, super::Value::Number(42.0));
                completed = true;
            }
            Err(error) => {
                assert!(
                    error.message.ends_with("allocation failed: Allocator"),
                    "{index}: {error:?}"
                );
                failures += 1;
            }
        }
        assert_eq!(instance.runtime.module, None, "allocation {index}");
        assert_eq!(instance.runtime.depth, 0, "allocation {index}");
        assert!(instance.runtime.loading.is_empty(), "allocation {index}");
        assert!(
            Rc::ptr_eq(&references, &instance.runtime.current_references),
            "allocation {index}"
        );
        assert_eq!(
            instance.call("load", &[], limits).unwrap(),
            super::Value::Number(42.0),
            "allocation {index}"
        );
        drop(instance);
        assert_eq!(ledger.live.get(), 0, "allocation {index}");
        if completed {
            break;
        }
    }
    assert!(completed && failures >= 8);
}
