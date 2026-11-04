import { startupSettings } from './pacing.mjs';
import { createMovementView } from './movement.mjs';
// Presentation and host scheduling only: Rust owns generation and query policy.
const $ = (id) => document.getElementById(id);
const metrics = ['submitted', 'applications', 'queued', 'queries', 'steps', 'overruns'];
const params = new URLSearchParams(location.search);
const result = window.contimeExample = { done: false, error: null, report: null, frames: 0 };
let BudgetDemo, demo, config, movementView, running = false, animationId = null;
let wantsToRun = false;
let activeMs = 0, previousTime = null, overruns = 0, duration = null;
function status(message) { $('state').textContent = message; $('state').hidden = message !== 'Failed'; }
function controls() {
    result.paused = !!demo && !running && !result.done && !result.error;
    result.pauseReason = result.paused ? (wantsToRun && document.hidden ? "hidden" : "manual") : null;
    movementView?.running(running);
    $('start').disabled = !BudgetDemo || running || result.done || !!result.error;
    $('start').textContent = demo ? 'Resume' : 'Start';
    $('pause').disabled = !running;
    $('reset').disabled = !BudgetDemo;
}
function fail(error) {
    wantsToRun = running = false;
    previousTime = null;
    cancelTick();
    result.error = String(error);
    $('error').textContent = `Run stopped: ${result.error}`;
    $('error').hidden = false;
    status('Failed');
    controls();
}
function measure() {
    const now = performance.now();
    if (previousTime !== null) activeMs += now - previousTime;
    previousTime = now;
}
function render(report) {
    $('movement-panel').hidden = false;
    if (report.movement) movementView?.draw(report.movement);
    else movementView?.drawDashboard(report, config.domain);
}

function cancelTick() {
    if (animationId !== null) cancelAnimationFrame(animationId);
    animationId = null;
}
function syncRunning() {
    const next = wantsToRun && !document.hidden && !result.done && !result.error;
    if (next !== running) {
        running = next;
        // Ignore all wall-clock time while suspended. In-flight work keeps its
        // original simulation deadline and remaining simulated delivery delay.
        previousTime = running ? performance.now() : null;
        if (running) schedule(); else cancelTick();
    }
    if (demo && !result.done && !result.error) status(running ? 'Running' : 'Paused');
    if (movementView && !document.hidden && !result.error) schedule();
    controls();
}
function pause() { wantsToRun = false; syncRunning(); }
document.addEventListener('visibilitychange', syncRunning);
window.addEventListener('keydown', (event) => {
    if (!demo || (event.key.toLowerCase() !== 'p' && event.code !== 'KeyP')
        || event.repeat || event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    if (wantsToRun) pause(); else resume();
}, { capture: true }); // Bevy consumes canvas keyboard events before they bubble.
function schedule() { if (animationId === null) animationId = requestAnimationFrame(tick); }
function tick() {
    animationId = null;
    if (document.hidden) { syncRunning(); return; }
    if (movementView?.takeReset()) {
        $('reset').click();
        $('start').click();
        return;
    }
    if (!running) { if (movementView) schedule(); return; }
    measure();
    try {
        const stopping = duration !== null && activeMs >= duration * 1000;
        const report = JSON.parse(demo.frame(activeMs, config.max_ms, config.max_steps, stopping));
        result.frames += 1;
        result.report = report;
        if (report.overrun) overruns += 1;
        render(report);
        measure();
        if (report.failure) {
            wantsToRun = running = false;
            previousTime = null;
            result.error = String(report.failure);
            controls();
            schedule();
            return;
        }
        if (report.done) {
            movementView?.finish();
            wantsToRun = running = false;
            previousTime = null;
            result.done = true;
            status('Completed · all work drained');
            controls();
            schedule();
        } else {
            status(stopping ? 'Draining' : 'Running');
            schedule();
        }
    } catch (error) { fail(error); }
}
$('movement-map').addEventListener('keydown', (event) => {
    const direction = ({w:'north',ArrowUp:'north',s:'south',ArrowDown:'south',a:'west',ArrowLeft:'west',d:'east',ArrowRight:'east',' ':'stop'})[event.key];
    if (direction && demo && running && config.domain === 'client-movement') { event.preventDefault(); try { demo.input(direction); } catch(error) { fail(error); } }
});
function resume() {
    if (running || result.done || result.error || !BudgetDemo) return;
    try {
        if (!demo) {
            demo = new BudgetDemo(JSON.stringify(config), config.domain);
        }
        wantsToRun = true;
        syncRunning();
    } catch (error) { fail(error); }
}
$('start').addEventListener('click', resume);
$('pause').addEventListener('click', pause);
$('reset').addEventListener('click', () => {
    wantsToRun = running = false;
    cancelTick();
    demo?.free(); demo = null;
    activeMs = overruns = 0; previousTime = null;
    Object.assign(result, { done: false, error: null, report: null, frames: 0 });
    $('error').hidden = true;
    [...metrics, 'visible', 'pending', 'throughput', 'target-rate'].forEach(id => { $(id).textContent = '0'; });
    $('query_ms').textContent = $('frame_ms').textContent = '0 ms';
    $('worker-rows').replaceChildren();
    $('values-panel').hidden = true;
    $('movement-panel').hidden = true;
    movementView?.reset();
    $('elapsed').textContent = '0 s'; $('values').textContent = 'Waiting for first query';
    status('Ready'); controls();
});
try {
    config = startupSettings(params);
    const movement = config.domain === 'client-movement';
    $('page').className = movement ? 'movement-example movement-game' : 'movement-example';
    $('diagnostics').open = false;
    $('diagnostics-title').hidden = true;
    $('run-title').hidden = true;
    $('movement-map').setAttribute('aria-label', movement ? 'Movement comparison; WASD or arrows move, Space stops, P pauses or resumes' : config.domain === 'chain-hash' ? 'Chain hash metrics; P pauses or resumes' : 'Counter metrics; P pauses or resumes');
    if (params.has('duration')) {
        duration = Number(params.get('duration'));
        if (!Number.isFinite(duration) || duration <= 0) throw new Error('duration must be a positive number of seconds');
    }
    controls();
    const bindings = await import('../pkg/contime_browser_example.js');
    await bindings.default(); BudgetDemo = bindings.BudgetDemo;
    const graphics = await import('../pkg/graphics/contime_graphics_example.js');
    await graphics.default();
    movementView = createMovementView(graphics);
    status('Ready'); controls();
    // The movement case is a graphical application, so no browser controls are needed.
    $('start').click();
} catch (error) { fail(error); }
