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
async function browserPage(search, report = null) {
    let frameCallback;
    const { readFile } = await import('node:fs/promises');
    const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
    const elements = new Map();
    for (const match of html.matchAll(/<[^>]+\bid="([^"]+)"[^>]*>/g)) {
        const tag = match[0];
        elements.set(match[1], { hidden: /\shidden\b/.test(tag),
            disabled: /\sdisabled\b/.test(tag), handlers: {},
            addEventListener(name, handler) { this.handlers[name] = handler; },
            replaceChildren() { this.textContent = ''; } });
    }
    const started = [];
    const graphicsEvents = [];
    globalThis.graphicsBindings = { default: async () => {}, start_graphics: () => graphicsEvents.push("start"),
        update_scene() { graphicsEvents.push("draw"); }, finish_scene: () => graphicsEvents.push("finish"), set_running: value => graphicsEvents.push(value), reset_scene: () => graphicsEvents.push("reset") };
    globalThis.window = {};
    globalThis.location = { search };
    globalThis.document = { getElementById: id => elements.get(id) };
    globalThis.requestAnimationFrame = callback => { frameCallback = callback; return 1; };
    globalThis.cancelAnimationFrame = () => {};
    elements.get('start').click = () => elements.get('start').handlers.click();
    globalThis.browserBindings = { default: async () => {}, BudgetDemo: class {
        constructor(json) { started.push(JSON.parse(json)); }
        free() {}
        frame() { return JSON.stringify(report); }
    } };
    const source = (await readFile(new URL('../ui/app.js', import.meta.url), 'utf8'))
        .replace("'./movement.mjs'", JSON.stringify(new URL('../ui/movement.mjs', import.meta.url).href))
        .replace("'./pacing.mjs'", JSON.stringify(new URL('../ui/pacing.mjs', import.meta.url).href))
        .replace("await import('../pkg/contime_browser_example.js')", 'globalThis.browserBindings')
        .replace("await import('../pkg/graphics/contime_graphics_example.js')", 'globalThis.graphicsBindings');
    await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}#${encodeURIComponent(search)}`);
    return { elements, started, graphicsEvents, result: window.contimeExample, runFrame: () => frameCallback() };
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
