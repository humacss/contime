//! Snapshot population/distribution helpers used by simulator assignment callbacks.
use super::Options;
use contime_simulator::Random;
use std::num::NonZeroU64;
pub fn assign_snapshot_ids(options: &Options, random: &mut Random, targets: &mut Vec<u128>) {
    assign_with_primary(options, random, targets, None);
}
pub fn assign_with_primary(options: &Options, random: &mut Random, targets: &mut Vec<u128>, primary: Option<u128>) {
    let count = target_count(options, random);
    if let Some(primary) = primary {
        // Hash groups keep one common primary; additional IDs follow cyclically,
        // retaining the previous hash example's O(selected targets) routing shape.
        targets.extend((0..count).map(|offset| (primary - 1 + offset as u128) % options.snapshots as u128 + 1));
        return;
    }
    if count == options.snapshots {
        targets.extend(1..=count as u128);
        return;
    }
    while targets.len() < count {
        let remaining = options.snapshots - targets.len();
        let skew = options.snapshot_skew;
        let rank = if skew == 0 {
            draw(random, remaining)
        } else if skew == 100 {
            0
        } else {
            let u = u128::from(random.next_u64());
            let denominator = 100 * (1_u128 << 64) - skew as u128 * u;
            (u * (100 - skew) as u128 * remaining as u128 / denominator) as usize
        };
        let mut id = rank as u128 + 1;
        for &used in targets.iter() {
            if used <= id {
                id += 1;
            }
        }
        targets.push(id);
        targets.sort_unstable();
    }
}

fn target_count(options: &Options, random: &mut Random) -> usize {
    let Some(average) = options.average_scaled else { return options.targets };
    if average == 1_000 || options.targets == 1 {
        return 1;
    }
    if average == options.targets as u64 * 1_000 {
        return options.targets;
    }
    let probability = (average - 1_000) as f64 / ((options.targets - 1) * 1_000) as f64;
    let log_failure = (-probability).ln_1p();
    let mut remaining = options.targets - 1;
    let mut count = 1;
    // Skip failed trials, rather than walking a potentially million-target cap.
    while remaining > 0 {
        let uniform = ((random.next_u64() >> 12) + 1) as f64 / 4_503_599_627_370_497.0;
        let failures = (uniform.ln() / log_failure).floor();
        if failures >= remaining as f64 {
            break;
        }
        remaining -= failures as usize + 1;
        count += 1;
    }
    count
}
fn draw(random: &mut Random, upper: usize) -> usize {
    // Caller chooses a nonempty pool; Options limits population to one million.
    random.below(NonZeroU64::new(upper as u64).expect("selected pool is nonempty")) as usize
}
