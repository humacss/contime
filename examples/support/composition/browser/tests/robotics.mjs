import assert from 'node:assert/strict';
import test from 'node:test';
import {startupSettings,readSettings} from '../ui/pacing.mjs';
import {createVisualizerView} from '../ui/movement.mjs';
test('robotics uses shared startup limits and an independent presentation bridge',()=>{
    const config=startupSettings(new URLSearchParams('domain=robotics&query_hz=10'));
    assert.equal(config.domain,'robotics');assert.equal(config.query_hz,10);
    assert.equal(config.clients,undefined);assert.equal(config.services,undefined);
    assert.ok(Object.isFrozen(config));assert.throws(()=>readSettings({...config,max_steps:0}));
    const calls=[];
    const view=createVisualizerView({start_graphics:()=>{},update_robotics:json=>calls.push(JSON.parse(json)),reset_scene:()=>calls.push('reset')},'robotics');
    view.draw({historical:{},current:{}});view.reset();assert.deepEqual(calls,[{historical:{},current:{}},'reset']);
});
