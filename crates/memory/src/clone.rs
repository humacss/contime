use std::mem::size_of;
use std::sync::Arc;

use crate::types::{MemoryAccount, MemoryKind, TrackedArc};

impl<T, M> TrackedArc<T, M>
where
    M: MemoryAccount,
{
    pub fn try_clone(&self) -> Result<Self, M::Error> {
        self.inner.memory.try_reserve(MemoryKind::Pointer, size_of::<Self>() as u64)?;
        Ok(Self { inner: Arc::clone(&self.inner) })
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use crate::{ConservativeSize, MemoryBudget, TrackedArc};

    struct Value(u64);

    impl ConservativeSize for Value {
        fn conservative_size(&self) -> u64 {
            64
        }
    }

    #[test]
    fn clone_reserves_only_one_additional_pointer() {
        let memory = MemoryBudget::new(1_000);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let allocation_bytes = memory.allocation_bytes();

        let clone = original.try_clone().unwrap();

        assert_eq!(memory.allocation_bytes(), allocation_bytes);
        assert_eq!(memory.pointer_bytes(), (size_of::<TrackedArc<Value>>() * 2) as u64);
        assert!(std::ptr::eq(original.as_ref(), clone.as_ref()));
        assert_eq!(clone.0, 7);
    }

    #[test]
    fn failed_clone_changes_neither_pointer_count_nor_value() {
        let sizing_memory = MemoryBudget::new(1_000);
        let sizing_value = TrackedArc::try_new(Value(7), sizing_memory.clone()).unwrap();
        let exact_limit = sizing_memory.used();
        drop(sizing_value);

        let memory = MemoryBudget::new(exact_limit);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let used = memory.used();

        assert!(original.try_clone().is_err());
        assert_eq!(memory.used(), used);
        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        assert_eq!(original.0, 7);
    }

    #[test]
    fn dropping_non_final_pointer_releases_only_pointer_bytes() {
        let memory = MemoryBudget::new(1_000);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let clone = original.try_clone().unwrap();
        let allocation_bytes = memory.allocation_bytes();

        drop(clone);

        assert_eq!(memory.allocation_bytes(), allocation_bytes);
        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        drop(original);
        assert_eq!(memory.used(), 0);
    }

    #[test]
    fn concurrent_pointer_drops_return_all_memory() {
        let memory = MemoryBudget::new(10_000);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let pointers = (0..32).map(|_| original.try_clone().unwrap()).collect::<Vec<_>>();
        let handles = pointers.into_iter().map(|pointer| std::thread::spawn(move || drop(pointer))).collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        drop(original);
        assert_eq!(memory.used(), 0);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }
}
