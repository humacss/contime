import assert from 'node:assert/strict';
import test from 'node:test';
import {startupSettings,readSettings} from '../ui/pacing.mjs';
import {createUptimeView} from '../ui/movement.mjs';
test('uptime startup keeps only applicable bounded fixed settings',()=>{
    const config=startupSettings(new URLSearchParams('domain=service-uptime&services=3&cadence_ms=250&latency_ms=1200'));
    assert.equal(config.min_delay_ms,250); assert.equal(config.services,3); assert.equal(config.cadence_ms,250); assert.equal(config.latency_ms,1200);
    assert.equal(config.clients,undefined); assert.equal(config.rate,undefined); assert.ok(Object.isFrozen(config));
    for(const [key,value] of [['services',17],['cadence_ms',0],['max_ms',NaN],['max_steps',0],['latency_ms',-1],['min_delay_ms',-1]]) assert.throws(()=>readSettings({...config,[key]:value}));
});
test('uptime rendering and pause reset lifetime use the shared graphics bridge',()=>{
    const calls=[];
    const view=createUptimeView({start_graphics:canvas=>calls.push(canvas),update_uptime:(json,reduced)=>calls.push([JSON.parse(json),reduced]),set_running:running=>calls.push(running),finish_scene:()=>calls.push('finish'),reset_scene:()=>calls.push('reset')});
    const report={services:[{id:1,reconciled:{downtime_ms:10},arrival_order:{downtime_ms:0}}]};
    view.draw(report); view.running(false);view.running(true);view.finish();view.reset();
    assert.deepEqual(calls,['#movement-map',[report,false],false,true,'finish','reset']);
});
