// Real WASI regression: native tests do not exercise Node's WASI stdin behavior.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const cwd=fileURLToPath(new URL('..',import.meta.url));
async function run(closed, quit=false) {
    const child=spawn(process.execPath,['run.mjs','--domain','client-movement','--clients','4','--input-hz','30',
        '--average-latency-ms','100','--jitter-ms','100','--trail-ms','200','--retention-ms','1000',
        '--duration',quit?'60':'1','--metrics-stdio','--control-stdin'],{cwd});
    let stdout='',stderr='';child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
    let quitSent=false;
    child.stdout.on('data',s=>{
        stdout+=s;
        const lines=stdout.split('\n');
        if(quit&&!quitSent&&lines.length>1) {
            const row=JSON.parse(lines.at(-2).split('\t')[1]);
            if(row.movement.updates>=2){child.stdin.write('q');quitSent=true;}
        }
    });child.stderr.on('data',s=>{stderr+=s;});
    if(closed) child.stdin.end('d'); else child.stdin.write('d');
    const timer=setTimeout(()=>child.kill(),10000);
    try {
        const code=await new Promise((resolve,reject)=>{child.on('error',reject);child.on('close',resolve);});
        assert.equal(code,0,stderr);
        const rows=stdout.trim().split('\n').map(line=>JSON.parse(line.split('\t')[1]));
        const last=rows.at(-1);
        assert.equal(last.done,true);assert.equal(last.metrics.outstanding,0);
        assert.equal(last.movement.keyboard_inputs,1);if(!quit) assert.ok(last.movement.late>0);
        assert.ok(last.movement.updates>1);assert.equal(last.movement.positions.length,4);
        assert.ok(last.movement.center[0]>0);
    } finally {clearTimeout(timer);child.stdin.destroy();if(child.exitCode===null)child.kill();}
}
test('Node WASI receives queued keyboard input before stdin EOF',()=>run(true));
test('Node WASI keeps progressing while the control pipe is open and idle',()=>run(false));

test('Node WASI quit command drains and validates without waiting for duration',()=>run(false,true));
