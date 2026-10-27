//! Execution selection is separate from the application definition and host UI.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Manual,
    Runtime,
}
impl Kind {
    pub fn select(args: impl IntoIterator<Item = String>, manual_default: bool) -> Result<Self, String> {
        let mut selected = if manual_default { Self::Manual } else { Self::Runtime };
        let mut args = args.into_iter();
        let mut seen = false;
        while let Some(arg) = args.next() {
            if arg != "--execution" {
                continue;
            }
            if seen {
                return Err("Specify --execution only once".into());
            }
            selected = match args.next().as_deref() {
                Some("manual") => Self::Manual,
                Some("runtime") => Self::Runtime,
                _ => return Err("--execution requires manual or runtime".into()),
            };
            seen = true;
        }
        Ok(selected)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_selection_is_independent_of_application() {
        let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Kind::select(args(&[]), false).unwrap(), Kind::Runtime);
        assert_eq!(Kind::select(args(&["--execution", "manual", "--query-hz", "1"]), false).unwrap(), Kind::Manual);
        assert_eq!(Kind::select(args(&["--execution", "runtime"]), true).unwrap(), Kind::Runtime);
        for args in [args(&["--execution"]), args(&["--execution", "threads"]), args(&["--execution", "manual", "--execution", "runtime"])]
        {
            assert!(Kind::select(args, false).is_err());
        }
    }
}
