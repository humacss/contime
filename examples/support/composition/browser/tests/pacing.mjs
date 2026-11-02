import test from 'node:test';
import assert from 'node:assert/strict';
import { parseRate, readSettings, startupSettings } from '../ui/pacing.mjs';
const values = { domain: 'counter', snapshots: '8', rate: 'Maximum', query_hz: '20', retention_ms: '1000', checkpoint_interval: '128', max_pending: '128', average_targets: '1.1', max_targets: '2', max_ms: '2', max_steps: '256' };
test('Maximum and numeric rates are parsed without host pacing', () => {
    for (const value of ['max', 'Maximum', 'MAXIMUM', '0']) assert.equal(parseRate(value), 0);
    assert.equal(parseRate('1000000000'), 1000000000);
    for (const value of ['-1', 'NaN', 'Infinity', '', '1000000001', '1.5']) assert.throws(() => parseRate(value));
});
test('startup settings are immutable and preserve decimal target averages', () => {
    const config = readSettings(values);
    assert.equal(config.average_targets, '1.1');
    assert.equal(config.snapshots, 8);
    assert.ok(Object.isFrozen(config));
    assert.throws(() => { config.snapshots = 16; });
});
test('invalid workload combinations and frame allowances are rejected', () => {
    for (const [key, value] of [['snapshots', '0'], ['query_hz', '0'], ['average_targets', '2.1'], ['max_targets', '9'], ['max_ms', 'Infinity'], ['max_ms', '0'], ['max_steps', '0'], ['domain', 'unknown']]) {
        assert.throws(() => readSettings({ ...values, [key]: value }), `${key}=${value}`);
    }
});

test('snapshot skew defaults to even and validates concentration settings', () => {
    assert.equal(readSettings(values).snapshot_skew, 0);
    for (const skew of [0, 50, 99, 100]) assert.equal(readSettings({ ...values, snapshot_skew: String(skew) }).snapshot_skew, skew);
    for (const skew of [-1, 101, 0.5, 'NaN']) assert.throws(() => readSettings({ ...values, snapshot_skew: skew }));
});

test('Maximum omits irrelevant pending settings, including invalid legacy values', () => {
    for (const max_pending of [undefined, '', '0', '-1', 'NaN', '10000001']) {
        const config = readSettings({ ...values, max_pending });
        assert.ok(!Object.hasOwn(config, 'max_pending'));
    }
});
test('numeric source windows are independent of snapshot fanout', () => {
    assert.equal(readSettings({ ...values, rate: '100' }).max_pending, 128);
    assert.equal(readSettings({ ...values, rate: '100', max_pending: '1' }).max_pending, 1);
    for (const max_pending of [undefined, '', '0', '-1', 'NaN', '10000001']) {
        assert.throws(() => readSettings({ ...values, rate: '100', max_pending }));
    }
});

// Load the actual host module with lightweight DOM and binding adapters. The
// startup parsing and run controls still execute unchanged.
async function browserPage(search, report = null, hidden = false) {
    let frameCallback, now = 0;
    const frameTimes = [];
    const inputs = [];
    const { readFile } = await import('node:fs/promises');
    const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
    const elements = new Map();
    for (const match of html.matchAll(/<[^>]+\bid="([^"]+)"[^>]*>/g)) {
        const tag = match[0];
        elements.set(match[1], { hidden: /\shidden\b/.test(tag),
            disabled: /\sdisabled\b/.test(tag), handlers: {},
            addEventListener(name, handler) { this.handlers[name] = handler; },
            setAttribute(name,value) { this[name]=value; },
            replaceChildren() { this.textContent = ''; } });
    }
    const started = [];
    const graphicsEvents = [];
    let pendingChoice, pendingNavigation;
    const lifecycle = [], frameCalls = [];
    let nextReport = report;
    globalThis.graphicsBindings = { default: async () => {}, start_graphics: () => graphicsEvents.push("start"),
        take_navigation() { const action = pendingNavigation; pendingNavigation = undefined; return action; },
        show_visualizations() { graphicsEvents.push("choices"); },
        take_visualization() { const choice = pendingChoice; pendingChoice = undefined; return choice; },
        update_scene() { graphicsEvents.push("draw"); }, update_uptime() { graphicsEvents.push("uptime"); }, finish_scene: () => graphicsEvents.push("finish"), set_running: value => graphicsEvents.push(value), reset_scene: () => graphicsEvents.push("reset") };
    globalThis.window = { handlers: {}, listenerOptions: {}, addEventListener(name, handler, options) { this.handlers[name] = handler; this.listenerOptions[name] = options; } };
    globalThis.location = { search };
    globalThis.document = { hidden, handlers: {}, listenerOptions: {}, addEventListener(name, handler, options) { this.handlers[name] = handler; this.listenerOptions[name] = options; }, getElementById: id => elements.get(id) };
    const document = globalThis.document;
    globalThis.browserNow = () => now;
    globalThis.requestAnimationFrame = callback => { frameCallback = callback; return 1; };
    globalThis.cancelAnimationFrame = () => { frameCallback = null; };
    elements.get('start').click = () => elements.get('start').handlers.click();
    globalThis.browserBindings = { default: async () => {}, BudgetDemo: class {
        constructor(json) { started.push(JSON.parse(json)); lifecycle.push('start'); }
        free() { lifecycle.push('free'); }
        input(direction) { inputs.push(direction); }
        frame(elapsed, maxMs, maxSteps, stopping) { frameTimes.push(elapsed); frameCalls.push({elapsed,maxMs,maxSteps,stopping}); return JSON.stringify(typeof nextReport === 'function' ? nextReport(frameCalls.at(-1)) : nextReport); }
    } };
    const source = (await readFile(new URL('../ui/app.js', import.meta.url), 'utf8'))
        .replaceAll("performance.now()", "globalThis.browserNow()")
        .replace("'./movement.mjs'", JSON.stringify(new URL('../ui/movement.mjs', import.meta.url).href))
        .replace("'./pacing.mjs'", JSON.stringify(new URL('../ui/pacing.mjs', import.meta.url).href))
        .replace("await import('../pkg/contime_browser_example.js')", 'globalThis.browserBindings')
        .replace("await import('../pkg/graphics/contime_graphics_example.js')", 'globalThis.graphicsBindings');
    await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}#${encodeURIComponent(search)}`);
    return { elements, started, graphicsEvents, frameTimes, frameCalls, lifecycle, inputs, navigate: action => { pendingNavigation = action; }, report: value => { nextReport = value; }, choose: choice => { pendingChoice = choice; }, listenerOptions: window.listenerOptions, result: window.contimeExample,
        runFrame: (elapsed = 16) => { now += elapsed; const callback = frameCallback; frameCallback = null; callback?.(); },
        visibility: (hidden, elapsed = 0) => { now += elapsed; document.hidden = hidden; document.handlers.visibilitychange?.(); },
        canvasKey: key => elements.get('movement-map').handlers.keydown({key, preventDefault() {}}),
        key: (key, options = {}) => window.handlers.keydown?.({key, preventDefault() {}, ...options}),
    };
}
test('browser has no settings form or editable configuration controls', async () => {
    const { elements, result } = await browserPage('');
    assert.equal(result.error, null);
    for (const id of ['settings','domain','rate','clients','client_spacing','movement_speed','max_pending']) assert.ok(!elements.has(id));
});
test('autostart URLs omit Maximum pending capacity and preserve numeric capacity', async () => {
    const maximum = await browserPage('?autostart=1&rate=0&max_pending=invalid');
    assert.equal(maximum.result.error, null);
    assert.equal(maximum.started.length, 1);
    assert.ok(!Object.hasOwn(maximum.started[0], 'max_pending'));
    const numeric = await browserPage('?autostart=1&rate=100&max_pending=64');
    assert.equal(numeric.result.error, null);
    assert.equal(numeric.started[0].max_pending, 64);
});

test('movement startup settings preserve latency, clients and processing budget',()=>{
    const movement={...values,domain:'client-movement',clients:'5',input_hz:'30',average_latency_ms:'50',jitter_ms:'300',trail_ms:'200',fade_ms:'500'};
    const settings=readSettings(movement);
    assert.equal(settings.average_latency_ms,50);assert.equal(settings.clients,5);assert.ok(Object.isFrozen(settings));
    assert.ok(!Object.hasOwn(settings,'rate'));
    for(const [key,value] of [['clients',0],['query_hz',61],['retention_ms',300],['average_latency_ms',-1]]) assert.throws(()=>readSettings({...movement,[key]:value}));
});

test('movement scale settings default, override and validate independently',()=>{
    const values={domain:'client-movement',clients:4,input_hz:5,query_hz:20,average_latency_ms:50,jitter_ms:0,trail_ms:200,fade_ms:1000,
        retention_ms:10000,checkpoint_interval:128,max_ms:1,max_steps:1000};
    const defaults=readSettings(values);assert.equal(defaults.client_spacing,16);assert.equal(defaults.movement_speed,6);
    const custom=readSettings({...values,client_spacing:9,movement_speed:10});assert.equal(custom.client_spacing,9);assert.equal(custom.movement_speed,10);
    for(const key of ['client_spacing','movement_speed']) {
        for(const value of [0,1000000]) assert.equal(readSettings({...values,[key]:value})[key],value);
        for(const value of [-1,0.5,1000001,'NaN']) assert.throws(()=>readSettings({...values,[key]:value}));
    }
});
test('movement URLs preserve spacing and speed through autostart',async()=>{
    const page=await browserPage('?autostart=1&domain=client-movement&client_spacing=9&movement_speed=10');
    assert.equal(page.result.error,null);assert.equal(page.started[0].client_spacing,9);assert.equal(page.started[0].movement_speed,10);
});

test('Reset clears results and the previous movement display', async () => {
    const { elements, result, graphicsEvents } = await browserPage('?domain=client-movement');
    assert.equal(result.error, null);
    for (const id of ['submitted', 'visible', 'pending', 'throughput', 'target-rate']) elements.get(id).textContent = '123';
    elements.get('movement-panel').hidden = false;
    elements.get('values-panel').hidden = false;
    elements.get('reset').handlers.click();
    assert.equal(elements.get('state').textContent, 'Ready');
    assert.equal(result.report, null);
    for (const id of ['submitted', 'visible', 'pending', 'throughput', 'target-rate']) assert.equal(elements.get(id).textContent, '0', id);
    assert.equal(elements.get('movement-panel').hidden, true);
    assert.equal(elements.get('values-panel').hidden, true);
    assert.ok(graphicsEvents.includes('reset'), 'previous Bevy presentation must be cleared');
});

test('movement starts without browser controls while counter retains its diagnostics', async () => {
    const movement = await browserPage('?domain=client-movement&focus-test=1');
    assert.equal(movement.result.error, null);
    assert.equal(movement.started.length, 1);
    assert.equal(movement.started[0].average_latency_ms, 500);
    assert.equal(movement.started[0].jitter_ms, 500);
    assert.equal(movement.elements.get('diagnostics').open, false);
    assert.equal(movement.elements.get('run-title').hidden, true);
    const counter = await browserPage('?domain=counter&focus-test=1');
    assert.equal(counter.elements.get('diagnostics').open, true);
    assert.equal(counter.elements.get('diagnostics-title').hidden, true);
});

test('movement is limited to five clients while counter scope stays independent',()=>{
    for (const clients of [1,5]) assert.equal(readSettings({...values,domain:'client-movement',clients,input_hz:2,
        average_latency_ms:50,jitter_ms:0,trail_ms:200,fade_ms:500}).clients,clients);
    assert.throws(()=>readSettings({...values,domain:'client-movement',clients:6,input_hz:2,
        average_latency_ms:50,jitter_ms:0,trail_ms:200,fade_ms:500}));
    assert.equal(readSettings({...values,snapshots:10000}).snapshots,10000);
});

// Scripted capture is explicit; ordinary interactive loads keep random generation.
test('recording choreography is opt-in', () => {
    assert.equal(startupSettings(new URLSearchParams('domain=client-movement')).showcase, undefined);
    assert.equal(startupSettings(new URLSearchParams('domain=client-movement&showcase=1')).showcase, true);
});

test('movement frames update graphics and reports without rebuilding hidden diagnostics', async () => {
    const report = {movement:{clients:5},submitted:10,applications:10,queued:0,queries:1,steps:1,
        completed:8,outstanding:2,elapsed:1,target_rate:5,workers_status:[],query_ms:0,frame_ms:0};
    const page = await browserPage('?domain=client-movement&hidden-diagnostics-test=1', report);
    page.elements.get('submitted').textContent = 'untouched';
    page.runFrame();
    assert.equal(page.result.error, null);
    assert.deepEqual(page.result.report, report);
    assert.ok(page.graphicsEvents.includes('draw'));
    assert.equal(page.elements.get('submitted').textContent, 'untouched');
});

test('counter frames retain their visible diagnostics', async () => {
    const report = {submitted:10,applications:10,queued:0,queries:1,steps:1,
        completed:8,outstanding:2,elapsed:1,target_rate:5,workers_status:[],query_ms:0,frame_ms:0};
    const page = await browserPage('?domain=counter&autostart=1&diagnostics-regression=1', report);
    page.runFrame();
    assert.equal(page.result.error, null);
    assert.equal(page.elements.get('submitted').textContent, '10');
    assert.equal(page.elements.get('visible').textContent, '8');
    assert.equal(page.elements.get('pending').textContent, '2');
    assert.ok(!page.graphicsEvents.includes('draw'));
});


const movementReport = { movement: { clients: 5 }, done: false };
test('hidden tabs freeze active time and resume without charging hours away', async () => {
    const page = await browserPage('?domain=client-movement&pause-hidden=1', movementReport);
    page.runFrame(16);
    const before = page.frameTimes.at(-1);
    page.visibility(true);
    assert.equal(page.graphicsEvents.at(-1), false);
    page.runFrame(3_600_000);
    assert.equal(page.frameTimes.length, 1);
    page.visibility(false);
    page.runFrame(16);
    assert.equal(page.frameTimes.at(-1), before + 16);
    assert.equal(page.result.error, null);
});
test('P pause is preserved across visibility changes and keyboard repeats', async () => {
    const page = await browserPage('?domain=client-movement&pause-manual=1', movementReport);
    assert.equal(page.listenerOptions.keydown.capture, true, 'P must run before Bevy consumes canvas keys');
    page.runFrame(16);
    page.key('p');
    assert.equal(page.graphicsEvents.at(-1), false);
    page.key('p', { repeat: true });
    page.visibility(true);
    page.visibility(false, 60_000);
    page.runFrame(16);
    assert.equal(page.frameTimes.length, 1);
    page.key('P');
    page.runFrame(16);
    assert.equal(page.frameTimes.length, 2);
    assert.equal(page.frameTimes[1], page.frameTimes[0] + 16);
});
test('hidden startup and reset never schedule unwanted background work', async () => {
    const page = await browserPage('?domain=client-movement&pause-start=1', movementReport, true);
    page.runFrame(60_000);
    assert.equal(page.frameTimes.length, 0);
    page.visibility(false);
    page.runFrame(16);
    assert.equal(page.frameTimes[0], 16);
    page.visibility(true);
    page.elements.get('reset').handlers.click();
    page.visibility(false, 60_000);
    page.runFrame(16);
    assert.equal(page.result.frames, 0);
    assert.equal(page.started.length, 1);
});
test('completed runs do not resume through visibility or P', async () => {
    const page = await browserPage('?domain=client-movement&pause-done=1', {...movementReport, done:true});
    page.runFrame();
    page.visibility(true);
    page.visibility(false, 60_000);
    page.key('p');
    page.runFrame();
    assert.equal(page.frameTimes.length, 1);
    assert.equal(page.result.done, true);
});


test('P preserves browser shortcuts and Space still stops movement only while running', async () => {
    const page = await browserPage('?domain=client-movement&pause-controls=1', movementReport);
    page.runFrame();
    page.key('p', { ctrlKey: true });
    page.canvasKey(' ');
    assert.deepEqual(page.inputs, ['stop']);
    page.key('p');
    page.canvasKey(' ');
    assert.deepEqual(page.inputs, ['stop']);
    page.key('p');
    page.canvasKey(' ');
    assert.deepEqual(page.inputs, ['stop', 'stop']);
});


test('uptime keeps its graphical view and excludes hidden and manually paused time', async()=>{
    const page=await browserPage('?domain=service-uptime&uptime-pause=1',{uptime:{services:[]},done:false});
    assert.equal(page.result.error,null); assert.equal(page.started.length,1);
    assert.equal(page.elements.get('run-title').hidden,true);
    assert.equal(page.elements.get('diagnostics').open,false);
    page.runFrame(16); assert.ok(page.graphicsEvents.includes('uptime'));
    assert.equal(page.elements.get('state').hidden,true);
    page.visibility(true);page.runFrame(3600000);page.visibility(false);page.runFrame(16);
    assert.deepEqual(page.frameTimes,[16,32]);
    page.key('p');page.runFrame(5000);page.visibility(true);page.visibility(false,10000);page.runFrame(16);
    assert.equal(page.frameTimes.length,2);
    page.key('P');page.runFrame(16);assert.deepEqual(page.frameTimes,[16,32,48]);
    page.elements.get('reset').handlers.click(); assert.equal(page.result.report,null);assert.equal(page.result.frames,0);
    page.visibility(true);page.visibility(false,10000);page.runFrame(16);assert.equal(page.result.frames,0);
});


test('visualizer chooser starts Bevy once without constructing or stepping a workload', async () => {
    const page = await browserPage('?domain=visualizer&autostart=1&chooser-idle=1');
    assert.equal(page.result.error, null);
    assert.equal(page.elements.get('movement-panel').hidden, false);
    assert.equal(page.elements.get('diagnostics').open, false);
    assert.equal(page.elements.get('start').disabled, true);
    page.runFrame(); page.runFrame();
    assert.equal(page.started.length, 0);
    assert.equal(page.frameTimes.length, 0);
    assert.equal(page.result.frames, 0);
    assert.equal(page.graphicsEvents.filter(event => event === 'start').length, 1);
    assert.ok(page.graphicsEvents.includes('choices'));
});

test('chooser creates only the selected case with its defaults and retained common budget', async () => {
    for (const [domain, jitter] of [['client-movement', 500], ['service-uptime', 800]]) {
        const report = domain === 'service-uptime' ? {uptime:{services:[]}} : movementReport;
        const page = await browserPage(`?domain=visualizer&max_ms=1.5&max_steps=1000&query_hz=12&retention_ms=12000&checkpoint_interval=64&choose=${domain}`, report);
        page.choose(domain); page.runFrame();
        assert.equal(page.result.error, null);
        assert.equal(page.started.length, 1);
        assert.equal(page.started[0].domain, domain);
        assert.equal(page.started[0].jitter_ms, jitter);
        assert.equal(page.started[0].max_ms, 1.5);
        assert.equal(page.started[0].max_steps, 1000);
        assert.equal(page.started[0].query_hz, 12);
        assert.equal(page.started[0].retention_ms, 12000);
        assert.equal(page.started[0].checkpoint_interval, 64);
        page.runFrame();
        assert.ok(page.graphicsEvents.includes(domain === 'service-uptime' ? 'uptime' : 'draw'));
        assert.equal(page.graphicsEvents.filter(event => event === 'start').length, 1);
    }
});

test('chooser reset returns to choices and can select another case in the same Bevy app', async () => {
    const page = await browserPage('?domain=visualizer&chooser-reset=1', movementReport);
    page.choose('client-movement'); page.runFrame(); page.runFrame();
    page.elements.get('reset').handlers.click();
    assert.equal(page.result.report, null);
    assert.equal(page.result.frames, 0);
    assert.equal(page.elements.get('movement-panel').hidden, false);
    page.runFrame();
    assert.equal(page.started.length, 1);
    page.choose('service-uptime'); page.runFrame();
    assert.equal(page.started.length, 2);
    assert.equal(page.started[1].domain, 'service-uptime');
    assert.equal(page.started[1].jitter_ms, 800);
    assert.equal(page.graphicsEvents.filter(event => event === 'start').length, 1);
});

test('hidden chooser waits for visibility before consuming a selection', async () => {
    const page = await browserPage('?domain=visualizer&chooser-hidden=1', movementReport, true);
    page.choose('client-movement'); page.runFrame();
    assert.equal(page.started.length, 0);
    page.visibility(false); page.runFrame();
    assert.equal(page.started.length, 1);
    assert.equal(page.frameTimes.length, 0);
    page.runFrame(); assert.equal(page.frameTimes[0], 16);
});


test('top-bar reset advances delivery time before freeing and restarts exactly the same settings and clock', async () => {
    const page = await browserPage('?domain=client-movement&clients=3&jitter_ms=123&max_ms=1&max_steps=7&navigation-reset=1', movementReport);
    page.runFrame(20);
    page.navigate('reset'); page.runFrame(20);
    assert.equal(page.started.length, 1);
    assert.deepEqual(page.lifecycle, ['start']);
    assert.equal(page.frameCalls.at(-1).stopping, true);
    assert.equal(page.frameCalls.at(-1).maxMs, 1);
    assert.equal(page.frameCalls.at(-1).maxSteps, 7);
    const stoppedAt = page.frameCalls.at(-1).elapsed;
    page.runFrame(1000);
    assert.equal(page.frameCalls.at(-1).elapsed, stoppedAt + 1000);
    page.report({...movementReport,done:true}); page.runFrame(1000);
    assert.deepEqual(page.lifecycle, ['start','free','start']);
    assert.deepEqual(page.started[1],page.started[0]);
    assert.equal(page.result.frames,0);
    assert.equal(page.result.paused,false);
    page.report(movementReport);page.runFrame(16);
    assert.equal(page.frameCalls.at(-1).elapsed,16);
    assert.equal(page.graphicsEvents.filter(event=>event==='start').length,1);
});

test('paused navigation advances delivery time while hidden tabs do no work', async () => {
    const page = await browserPage('?domain=service-uptime&navigation-paused=1', {uptime:{services:[]},done:false});
    page.runFrame(16);page.key('p');
    const frames=page.result.frames;
    page.runFrame(2000);assert.equal(page.result.frames,frames);
    page.navigate('menu');page.runFrame(1000);
    assert.equal(page.frameCalls.at(-1).elapsed,16);
    assert.equal(page.frameCalls.at(-1).stopping,true);
    page.visibility(true);const drained=page.frameCalls.length;page.runFrame(60000);
    assert.equal(page.frameCalls.length,drained);
    page.report({uptime:{services:[]},done:true});page.visibility(false);page.runFrame(16);
    assert.equal(page.frameCalls.at(-1).elapsed,32);
    assert.deepEqual(page.lifecycle,['start','free']);
    assert.equal(page.result.frames,0);
    assert.equal(page.elements.get('movement-panel').hidden,false);
    page.runFrame();assert.equal(page.started.length,1);
});

test('finished direct cases can reset or go back and a new choice uses fresh case defaults', async () => {
    const page=await browserPage('?domain=service-uptime&jitter_ms=999&services=2&max_steps=99&navigation-finished=1',{uptime:{services:[]},done:true});
    page.runFrame();assert.equal(page.result.done,true);
    page.navigate('reset');page.runFrame();
    assert.equal(page.started.length,2);assert.deepEqual(page.started[0],page.started[1]);
    page.runFrame();assert.equal(page.result.done,true);
    const frames=page.frameCalls.length;
    page.navigate('menu');page.runFrame();
    assert.equal(page.frameCalls.length,frames,'completed session needs no more drain frames');
    assert.equal(page.started.length,2);
    page.choose('client-movement');page.runFrame();
    assert.equal(page.started.length,3);
    assert.equal(page.started[2].jitter_ms,500);
    assert.equal(page.started[2].max_steps,99);
    assert.equal(page.started[2].services,undefined);
    assert.equal(page.graphicsEvents.filter(event=>event==='start').length,1);
});


test('delayed movement delivery must become due before navigation can release the old workload', async () => {
    let stopTime;
    const page = await browserPage('?domain=client-movement&average_latency_ms=500&jitter_ms=500&delivery-drain=1', frame => {
        if (!frame.stopping) return movementReport;
        stopTime ??= frame.elapsed;
        return {...movementReport,done:frame.elapsed >= stopTime + 1000};
    });
    page.runFrame(16);page.key('p');page.runFrame(5000);
    page.navigate('menu');page.runFrame(16);
    assert.deepEqual(page.lifecycle,['start']);
    page.runFrame(400);assert.deepEqual(page.lifecycle,['start']);
    page.visibility(true);page.runFrame(60000);
    page.visibility(false);page.runFrame(400);
    assert.equal(page.frameCalls.at(-1).elapsed,stopTime+800);
    assert.deepEqual(page.lifecycle,['start']);
    page.runFrame(200);
    assert.deepEqual(page.lifecycle,['start','free']);
    assert.equal(page.result.report,null);
    page.choose('service-uptime');page.runFrame(16);
    assert.deepEqual(page.lifecycle,['start','free','start']);
    assert.equal(page.graphicsEvents.filter(event=>event==='start').length,1);
});
