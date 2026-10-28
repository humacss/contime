import assert from 'node:assert/strict';
import test from 'node:test';
import { createMovementView } from '../ui/movement.mjs';

test('browser forwards queried reports unchanged to the shared Bevy renderer', () => {
    const calls = [];
    const view = createMovementView({ start_graphics: canvas => calls.push(canvas),
        update_scene: (json, reduced) => calls.push([JSON.parse(json), reduced]),
        set_running() {}, finish_scene() {}, reset_scene() {} });
    const report = { arena_half_extent: [48, 28], positions: [{ id: 1, position: [4, 2] }], trails: [] };
    const original = structuredClone(report);
    view.draw(report);
    assert.equal(calls[0], '#movement-map');
    assert.deepEqual(calls[1], [original, false]);
    assert.deepEqual(report, original);
});

test('pause and reset reach Bevy without modifying workload settings', () => {
    const calls = [];
    const view = createMovementView({ start_graphics() {}, update_scene() {},
        set_running: value => calls.push(value), finish_scene: () => calls.push('finish'), reset_scene: () => calls.push('reset') });
    view.running(true); view.running(false); view.finish(); view.reset();
    assert.deepEqual(calls, [true, false, 'finish', 'reset']);
});
