// Browser adapters only. All graphical hosts share the Rust/Bevy renderer.
export function createVisualizerView(bindings, domain = 'client-movement') {
    bindings.start_graphics('#movement-map');
    return {
        draw(report) {
            const reduced = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
            const update = domain === 'robotics' ? bindings.update_robotics : domain === 'account-balances' ? bindings.update_accounts : domain === 'service-uptime' ? bindings.update_uptime : bindings.update_scene;
            update(JSON.stringify(report), reduced);
        },
        select(value) { domain = value; },
        showChoices() { bindings.show_visualizations(); },
        takeNavigation() { return bindings.take_navigation(); },
        takeChoice() { return bindings.take_visualization(); },
        running(value) { bindings.set_running(value); },
        finish() { bindings.finish_scene(); },
        reset() { bindings.reset_scene(); },
    };
}
export function createMovementView(bindings) {
    return createVisualizerView(bindings, 'client-movement');
}
export function createUptimeView(bindings) {
    return createVisualizerView(bindings, 'service-uptime');
}
