//! Case explanations built from the actual workload settings.
use crate::{bridge::State, catalog::Visualization};
pub(crate) fn description(active: Visualization, state: &State) -> String {
    match active {
        Visualization::Movement => {
            let cadence = match state.report.as_ref().and_then(|report| report.bot_hz) {
                Some(1) => "All bots turn on the same one-second ticks.".into(),
                Some(hz) => format!("All bots turn on the same ticks, {hz} times per second."),
                None => "Bots follow square routes with regularly timed turns.".into(),
            };
            format!(
                "{cadence} The gray vertical lines mark one-second intervals.\n\nEach bot follows a dashed gray square, but independent network delays make its turns arrive at different times.\n\nOn the left, ConTime uses action timestamps to reconcile late turns: a bot can briefly overshoot, then return to its square. On the right, turns happen only on arrival, so paths drift and deform. Move your client with WASD or arrows; Space stops."
            )
        }
        Visualization::Robotics => "Wheel motion updates a position-and-heading estimate ten times per second. A slip patch makes the wheels overestimate travel. Camera jobs describe earlier captures but finish late, sometimes out of order.\n\nLeft: apply each observation at capture time and replay subsequent motion and planning. Right: apply that same observation at the current delivery time. Both use all results.\n\nGray is simulated physical truth; colored robots are estimates. Dashed routes are proposals, not executed commands. The robot follows the same scripted physical route in both maps. P pauses.".into(),
        Visualization::Accounts => "Both views receive the same deposits, payment requests and retries. Delays can make a later payment arrive first.\n\nConTime re-evaluates the account at the original action times; the other view decides in arrival order. Retrying an unchanged transaction keeps its ID and does not spend or credit money twice on either side.\n\nPayment decisions are provisional. This illustrates reconciliation and idempotency, not settlement or distributed double-spend prevention.".into(),
        Visualization::Uptime => {
            let cadence = match state.uptime.as_ref().and_then(|report| report.cadence_ms) {
                Some(1000) => "Each service produces one health event every second.".into(),
                Some(ms) => format!("Each service produces one health event every {ms} ms."),
                None => "Each service produces health events at a regular cadence.".into(),
            };
            format!(
                "{cadence} The gray vertical lines mark one-second intervals.\n\nNetwork delays make reports arrive late or out of order. Applying them only when they arrive can put outages at the wrong times and give incorrect uptime and downtime totals.\n\nConTime uses the original timestamps to reconcile earlier state. Compare the corrected history above with arrival-order history below, and their totals on the right."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn movement_explanation_matches_configured_shared_bot_ticks() {
        let bridge = crate::bridge::Bridge::default();
        let mut report = crate::animation::tests::scene(0.0);
        report.bot_hz = Some(2);
        bridge.publish(report).unwrap();
        let description = description(Visualization::Movement, &bridge.snapshot());
        assert!(description.contains("2 times per second"));
        assert!(description.contains("one-second intervals"));
        assert!(!description.contains("same one-second ticks"));
    }
    #[test]
    fn uptime_explanation_uses_configured_cadence_without_changing_the_second_grid() {
        let mut state = State::default();
        state.uptime = Some(std::sync::Arc::new(crate::uptime_report::Report { cadence_ms: Some(250), ..Default::default() }));
        let description = description(Visualization::Uptime, &state);
        assert!(description.contains("every 250 ms"));
        assert!(description.contains("one-second intervals"));
        assert!(!description.contains("every second."));
    }
}
