use std::mem::{size_of, size_of_val};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::types::{Allocation, ConservativeSize, MemoryAccount, MemoryKind, TrackedArc};

fn conservative_allocation_bytes<T, M>(value: &T) -> u64
where
    T: ConservativeSize,
    M: MemoryAccount,
{
    let value_bytes = value.conservative_size().max(size_of_val(value) as u64);
    let fixed_fields = size_of::<Allocation<T, M>>().saturating_sub(size_of::<T>()) as u64;
    let arc_counters = size_of::<AtomicUsize>().saturating_mul(2) as u64;
    value_bytes.saturating_add(fixed_fields).saturating_add(arc_counters)
}

impl<T, M> TrackedArc<T, M>
where
    T: ConservativeSize,
    M: MemoryAccount,
{
    pub fn try_new(value: T, memory: M) -> Result<Self, M::Error> {
        let allocation_bytes = conservative_allocation_bytes::<T, M>(&value);
        let pointer_bytes = size_of::<Self>() as u64;
        memory.try_reserve(MemoryKind::Allocation, allocation_bytes)?;
        if let Err(error) = memory.try_reserve(MemoryKind::Pointer, pointer_bytes) {
            memory.release(MemoryKind::Allocation, allocation_bytes);
            return Err(error);
        }
        Ok(Self { inner: Arc::new(Allocation { value, memory, allocation_bytes }) })
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use crate::{ConservativeSize, MemoryBudget, TrackedArc};

    #[derive(Debug, Eq, PartialEq)]
    struct Value(u64);

    impl ConservativeSize for Value {
        fn conservative_size(&self) -> u64 {
            64
        }
    }

    #[test]
    fn first_pointer_charges_allocation_and_pointer() {
        let memory = MemoryBudget::new(1_000);
        let value = TrackedArc::try_new(Value(7), memory.clone()).unwrap();

        assert_eq!(*value, Value(7));
        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        assert!(memory.allocation_bytes() >= 64);
        assert_eq!(memory.used(), memory.allocation_bytes() + memory.pointer_bytes());
    }

    #[test]
    fn dropping_final_pointer_releases_everything() {
        let memory = MemoryBudget::new(1_000);
        let value = TrackedArc::try_new(Value(7), memory.clone()).unwrap();

        drop(value);

        assert_eq!(memory.used(), 0);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn pointer_failure_rolls_back_allocation_reservation() {
        let sizing_memory = MemoryBudget::new(1_000);
        let sizing_value = TrackedArc::try_new(Value(7), sizing_memory.clone()).unwrap();
        let allocation_bytes = sizing_memory.allocation_bytes();
        drop(sizing_value);

        let memory = MemoryBudget::new(allocation_bytes);
        assert!(TrackedArc::try_new(Value(7), memory.clone()).is_err());
        assert_eq!(memory.used(), 0);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn allocation_failure_never_reserves_a_pointer() {
        let memory = MemoryBudget::new(1);

        assert!(TrackedArc::try_new(Value(7), memory.clone()).is_err());
        assert_eq!(memory.used(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn tracked_pointer_is_one_machine_pointer_wide() {
        assert_eq!(size_of::<TrackedArc<Value>>(), size_of::<usize>());
    }
}
