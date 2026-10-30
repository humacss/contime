# Time

Shared `Timestamp` and `AdvanceTime` contracts. `Timestamp::minimum` is the explicit domain lower bound; `Default` has no ordering meaning. Signed integers support their entire range, including MIN and negative timestamps. Retention intervals must be nonnegative. The default `time` feature supplies implementations for all Rust integer types. With it disabled, the traits remain available for application types.
