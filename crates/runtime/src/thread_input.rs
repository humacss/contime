use crate::contime_core::Input;

/// An input that can cross the managed runtime's thread boundaries.
/// Manual processing and Core require only [`Input`].
pub trait ThreadInput: Input<Time: Send + Sync> + Send + Sync {}
impl<I> ThreadInput for I
where
    I: Input + Send + Sync,
    I::Time: Send + Sync,
{
}
