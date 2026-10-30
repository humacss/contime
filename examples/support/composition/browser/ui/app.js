import { startupSettings } from './pacing.mjs';
import { createMovementView } from './movement.mjs';
// Presentation and host scheduling only: Rust owns generation and query policy.
const $ = (id) => document.getElementById(id);
const metrics = ['submitted', 'applications', 'queued', 'queries', 'steps', 'overruns'];
const params = new URLSearchParams(location.search);
const result = window.contimeExample = { done: false, error: null, report: null, frames: 0 };
let BudgetDemo, demo, config, movementView, running = false, animationId = null;
let activeMs = 0, previousTime = null, overruns = 0, duration = null;
function status(message) { $('state').textContent = message; $('state').hidden = config?.domain === 'client-movement' && message !== 'Failed'; }
function controls() {
    movementView?.running(running);
    $('start').disabled = !BudgetDemo || running || result.done || !!result.error;
    $('start').textContent = demo ? 'Resume' : 'Start';
    $('pause').disabled = !running;
    $('reset').disabled = !BudgetDemo;
}
function fail(error) {
    running = false;
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
const integer = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });
function metric(value) { return value === null || value === undefined ? '—' : integer.format(value); }
function renderWorkers(workers) {
    const rows = workers.map(worker => {
        const row = document.createElement('tr');
        for (const key of ['id', 'snapshots', 'total_inserted', 'total_replayed', 'avg_inserted_per_second', 'avg_replayed_per_second']) {
            const cell = document.createElement('td');
            cell.textContent = metric(worker[key]);
            cell.title = cell.textContent;
            row.appendChild(cell);
        }
        return row;
    });
    $('worker-rows').replaceChildren(...rows);
}
function render(report) {
    $('movement-panel').hidden = !report.movement;
    if (report.movement) {
        movementView?.draw(report.movement);
        return; // This case uses only the shared graphical view.
    }
    for (const id of metrics) $(id).textContent = metric(id === 'overruns' ? overruns : report[id]);
    $('visible').textContent = metric(report.completed);
    $('pending').textContent = metric(report.outstanding);
    const completed = report.completed;
    $('throughput').textContent = metric(report.elapsed ? completed / report.elapsed : 0);
    $('target-rate').textContent = metric(report.target_rate);
    $('values-panel').hidden = !report.hashes?.length;
    renderWorkers(report.workers_status);
    $('query_ms').textContent = `${report.query_ms.toFixed(3)} ms`;
    $('frame_ms').textContent = `${report.frame_ms.toFixed(3)} ms`;
    $('elapsed').textContent = `${report.elapsed.toFixed(2)} s`;
    $('values').replaceChildren(...(report.hashes || []).map(snapshot => {
        const row = document.createElement('p');
        const title = document.createElement('strong');
        title.textContent = `Snapshot ${snapshot.id}`;
        const latest = document.createElement('code');
        latest.textContent = `Latest (${metric(snapshot.latest_count)}): ${snapshot.latest_hash}`;
        const verified = document.createElement('code');
        verified.textContent = `Verified (${metric(snapshot.verified_count)}): ${snapshot.verified_hash}`;
        row.append(title, document.createElement('br'), latest, document.createElement('br'), verified);
        return row;
    }));
}

function schedule() { if (animationId === null) animationId = requestAnimationFrame(tick); }
function tick() {
    animationId = null;
    if (!running) return;
    measure();
    try {
        const stopping = duration !== null && activeMs >= duration * 1000;
        const report = JSON.parse(demo.frame(activeMs, config.max_ms, config.max_steps, stopping));
        result.frames += 1;
        result.report = report;
        if (report.overrun) overruns += 1;
        render(report);
        measure();
        if (report.done) {
            movementView?.finish();
            running = false;
            result.done = true;
            status('Completed · all work drained');
            controls();
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
$('start').addEventListener('click', () => {
    if (running || result.done || result.error || !BudgetDemo) return;
    try {
        if (!demo) {
            demo = new BudgetDemo(JSON.stringify(config), config.domain);
        }
        running = true;
        previousTime = performance.now();
        controls();
        schedule();
    } catch (error) { fail(error); }
});
$('pause').addEventListener('click', () => {
    measure(); running = false; previousTime = null;
    if (animationId !== null) cancelAnimationFrame(animationId);
    animationId = null;
    status('Paused'); controls();
});
$('reset').addEventListener('click', () => {
    running = false;
    if (animationId !== null) cancelAnimationFrame(animationId);
    animationId = null;
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
    $('page').className = movement ? 'movement-example' : '';
    $('diagnostics').open = !movement;
    $('diagnostics-title').hidden = !movement;
    $('run-title').hidden = movement;
    if (params.has('duration')) {
        duration = Number(params.get('duration'));
        if (!Number.isFinite(duration) || duration <= 0) throw new Error('duration must be a positive number of seconds');
    }
    controls();
    const bindings = await import('../pkg/contime_browser_example.js');
    await bindings.default(); BudgetDemo = bindings.BudgetDemo;
    if (config.domain === 'client-movement') {
        const graphics = await import('../pkg/graphics/contime_graphics_example.js');
        await graphics.default();
        movementView = createMovementView(graphics);
    }
    status('Ready'); controls();
    // The movement case is a graphical application, so no browser controls are needed.
    if (movement || params.get('autostart') === '1') $('start').click();
} catch (error) { fail(error); }
