// Browser adapter only. All graphical hosts use the same Rust/Bevy renderer.
export function createMovementView(bindings) {
    bindings.start_graphics('#movement-map');
    return {
        draw(report) {
            const reduced = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
            bindings.update_scene(JSON.stringify(report), reduced);
        },
        running(value) { bindings.set_running(value); },
        finish() { bindings.finish_scene(); },
        reset() { bindings.reset_scene(); },
    };
}
