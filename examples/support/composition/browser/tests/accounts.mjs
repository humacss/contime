import assert from 'node:assert/strict';
import test from 'node:test';
import {startupSettings,readSettings} from '../ui/pacing.mjs';
import {createVisualizerView} from '../ui/movement.mjs';
test('account visualization keeps shared settings and uses its own report bridge',()=>{
    const settings=startupSettings(new URLSearchParams('domain=account-balances&query_hz=5'));
    assert.equal(settings.query_hz,5);
    assert.equal(settings.services,undefined);
    assert.equal(settings.clients,undefined);
    assert.ok(Object.isFrozen(settings));
    assert.throws(()=>readSettings({...settings,query_hz:0}));
    const calls=[];
    const view=createVisualizerView({start_graphics:()=>{},update_accounts:json=>calls.push(JSON.parse(json))},'account-balances');
    view.draw({balance:40,arrival_balance:60});
    assert.deepEqual(calls,[{balance:40,arrival_balance:60}]);
});
