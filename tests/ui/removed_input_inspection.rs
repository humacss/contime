use contime::{InputJournalEntry, TestInputLanes, TestSnapshotContime};

fn accepts_removed_entry(_entry: InputJournalEntry<TestInputLanes>) {}

fn main() {
    let contime = TestSnapshotContime::new(1, 1_024);
    let _ = contime.inspect_inputs(..);
    let _ = accepts_removed_entry;
}
