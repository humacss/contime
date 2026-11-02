import { startupSettings } from './pacing.mjs';
import { createVisualizerView } from './movement.mjs';
// Presentation and host scheduling only: Rust owns generation and query policy.
const $ = (id) => document.getElementById(id);
const metrics = ['submitted', 'applications', 'queued', 'queries', 'steps', 'overruns'];
const graphical = (domain) => ['client-movement','service-uptime','account-balances','robotics','visualizer'].includes(domain);
const params = new URLSearchParams(location.search);
const result = window.contimeExample = { done: false, error: null, report: null, frames: 0 };
let BudgetDemo, demo, config, movementView, running = false, animationId = null;
const chooserLaunch = params.get('domain') === 'visualizer';
let choosing = chooserLaunch;
let wantsToRun = false;
let navigation = null;
let drainMs = 0, drainPreviousTime = null;
let activeMs = 0, previousTime = null, overruns = 0, duration = null;
function status(message) { $('state').textContent = message; $('state').hidden = graphical(config?.domain) && message !== 'Failed'; }
function controls() {
    result.paused = !!demo && !running && !result.done && !result.error;
    result.pauseReason = result.paused ? (wantsToRun && document.hidden ? "hidden" : "manual") : null;
    movementView?.running(running);
    $('start').disabled = choosing || !BudgetDemo || running || result.done || !!result.error;
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
    $('movement-panel').hidden = !(report.movement || report.uptime || report.accounts || report.robotics);
    if (report.movement || report.uptime || report.accounts || report.robotics) {
        movementView?.draw(report.movement || report.uptime || report.accounts || report.robotics);
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

function cancelTick() {
    if (animationId !== null) cancelAnimationFrame(animationId);
    animationId = null;
}
function syncRunning() {
    if (navigation) drainPreviousTime = document.hidden ? null : performance.now();
    const next = wantsToRun && !navigation && !document.hidden && !result.done && !result.error;
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
    if (navigation || !graphical(config?.domain) || !demo || (event.key.toLowerCase() !== 'p' && event.code !== 'KeyP')
        || event.repeat || event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    if (wantsToRun) pause(); else resume();
}, { capture: true }); // Bevy consumes canvas keyboard events before they bubble.
function schedule() { if (animationId === null) animationId = requestAnimationFrame(tick); }
// Presentation controls keep polling while the workload is paused or finished.
// They never count as workload frames or advance the simulation clock.
function tick() {
    animationId = null;
    if (document.hidden || result.error) return;
    try {
        const action = movementView?.takeNavigation();
        if (!navigation && action !== undefined) {
            if (!['reset', 'menu'].includes(action)) throw new Error('Invalid navigation');
            if (running) measure();
            navigation = action;
            drainMs = activeMs;
            drainPreviousTime = performance.now();
            wantsToRun = running = false;
            previousTime = null;
            controls();
        }
        if (navigation) {
            if (demo && !result.done) {
                // Stop freezes the case's production horizon, but delayed
                // inputs still need a visible host clock to reach delivery.
                const now = performance.now();
                if (drainPreviousTime !== null) drainMs += now - drainPreviousTime;
                drainPreviousTime = now;
                const report = JSON.parse(demo.frame(drainMs, config.max_ms, config.max_steps, true));
                result.frames += 1;
                result.report = report;
                if (!report.done) return;
            }
            const action = navigation;
            navigation = null;
            clearSession();
            if (action === 'menu') {
                choosing = true;
                config = { domain: 'visualizer' };
                movementView.showChoices();
            } else {
                choosing = false;
                movementView.reset();
            }
            $('movement-panel').hidden = false;
            configurePresentation();
            if (action === 'reset') resume(); else controls();
            return;
        }
        if (choosing) {
            const choice = movementView.takeChoice();
            if (choice !== undefined) {
                if (!['client-movement', 'service-uptime', 'account-balances', 'robotics'].includes(choice)) throw new Error('Invalid visualization');
                // A fresh choice keeps common host settings only. Settings from
                // the previous case belong exclusively to that case's reset.
                const selected = new URLSearchParams();
                for (const key of ['query_hz', 'retention_ms', 'checkpoint_interval', 'max_ms', 'max_steps']) {
                    if (params.has(key)) selected.set(key, params.get(key));
                }
                selected.set('domain', choice);
                config = startupSettings(selected);
                choosing = false;
                movementView.select(choice);
                configurePresentation();
                resume();
            }
            return;
        }
        if (!running) return;
        measure();
        const stopping = duration !== null && activeMs >= duration * 1000;
        const report = JSON.parse(demo.frame(activeMs, config.max_ms, config.max_steps, stopping));
        result.frames += 1;
        result.report = report;
        if (report.overrun) overruns += 1;
        render(report);
        measure();
        if (report.done) {
            movementView?.finish();
            wantsToRun = running = false;
            previousTime = null;
            result.done = true;
            status('Completed · all work drained');
            controls();
        } else status(stopping ? 'Draining' : 'Running');
    } catch (error) { fail(error); }
    finally {
        if (!document.hidden && !result.error && (movementView || running)) schedule();
    }
}
$('movement-map').addEventListener('keydown', (event) => {
    const direction = ({w:'north',ArrowUp:'north',s:'south',ArrowDown:'south',a:'west',ArrowLeft:'west',d:'east',ArrowRight:'east',' ':'stop'})[event.key];
    if (direction && demo && running && config.domain === 'client-movement') { event.preventDefault(); try { demo.input(direction); } catch(error) { fail(error); } }
});
function resume() {
    if (choosing || navigation || running || result.done || result.error || !BudgetDemo) return;
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
function clearSession() {
    wantsToRun = running = false;
    demo?.free(); demo = null;
    activeMs = overruns = drainMs = 0; previousTime = drainPreviousTime = null;
    Object.assign(result, { done: false, error: null, report: null, frames: 0 });
    $('error').hidden = true;
    [...metrics, 'visible', 'pending', 'throughput', 'target-rate'].forEach(id => { $(id).textContent = '0'; });
    $('query_ms').textContent = $('frame_ms').textContent = '0 ms';
    $('worker-rows').replaceChildren();
    $('values-panel').hidden = true;
    $('elapsed').textContent = '0 s'; $('values').textContent = 'Waiting for first query';
}
$('reset').addEventListener('click', () => {
    cancelTick();
    navigation = null;
    clearSession();
    choosing = chooserLaunch;
    if (choosing) config = { domain: 'visualizer' };
    $('movement-panel').hidden = !choosing;
    if (choosing) movementView?.showChoices(); else movementView?.reset();
    configurePresentation();
    if (movementView && !document.hidden) schedule();
    $('elapsed').textContent = '0 s'; $('values').textContent = 'Waiting for first query';
    status('Ready'); controls();
});
function configurePresentation() {
    const movement = graphical(config.domain);
    $('page').className = movement ? 'movement-example' : '';
    $('diagnostics').open = !movement;
    $('diagnostics-title').hidden = !movement;
    $('run-title').hidden = movement;
    if (movement) {
        $('movement-title').textContent = choosing ? 'Choose a visualization' : config.domain === 'robotics' ? 'Robotics / delayed perception' : config.domain === 'account-balances' ? 'Account balances' : config.domain === 'service-uptime' ? 'Service uptime' : 'Client movement';
        $('movement-map').setAttribute('aria-label', choosing ? 'Choose Client movement, Service uptime, Account balances or Robotics' : config.domain === 'robotics' ? 'Robot estimates from capture-time replay and current-time observations; P pauses or resumes' : config.domain === 'account-balances' ? 'Account balances and transaction delivery timeline; P pauses or resumes' : config.domain === 'service-uptime' ? 'Service health histories and delivery timeline; P pauses or resumes' : 'Movement arena and delivery timeline; WASD or arrows move, Space stops, P pauses or resumes');
    }
}
try {
    config = choosing ? { domain: 'visualizer' } : startupSettings(params);
    configurePresentation();
    if (params.has('duration')) {
        duration = Number(params.get('duration'));
        if (!Number.isFinite(duration) || duration <= 0) throw new Error('duration must be a positive number of seconds');
    }
    controls();
    const bindings = await import('../pkg/contime_browser_example.js');
    await bindings.default(); BudgetDemo = bindings.BudgetDemo;
    if (graphical(config.domain)) {
        const graphics = await import('../pkg/graphics/contime_graphics_example.js');
        await graphics.default();
        movementView = createVisualizerView(graphics, config.domain);
        if (!document.hidden) schedule();
        if (choosing) {
            $('movement-panel').hidden = false;
            movementView.showChoices();
            if (!document.hidden) schedule();
        }
    }
    status('Ready'); controls();
    // Graphical cases run through the shared canvas and its controls.
    if (!choosing && (graphical(config.domain) || params.get('autostart') === '1')) $('start').click();
} catch (error) { fail(error); }
