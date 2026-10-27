import test from 'node:test';
import assert from 'node:assert/strict';
import { drawMovement } from '../ui/movement.mjs';

test('opaque current points cover old paths at exactly the same canvas coordinates', () => {
    const draws = [];
    const ctx = {
        fillRect(...bounds) { draws.push({bounds, color: this.fillStyle, alpha: this.globalAlpha}); },
        fillText() {},
    };
    const canvas = {clientWidth: 799, getContext: () => ctx};
    drawMovement(canvas, {center: [12.3, -4.7], positions: [], trails: [
        {position: [13.4, -2.3], current: true, corrected: false, opacity: 1},
        {position: [13.4, -2.3], current: false, corrected: true, opacity: 0.7},
    ]});
    const [old, current] = draws.slice(1);
    assert.equal(old.color, '#e9aa3a');
    assert.equal(current.color, '#4bd5dd');
    assert.equal(current.alpha, 1);
    assert.deepEqual(current.bounds, old.bounds);
});
