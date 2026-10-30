// Input parsing only. The shared Rust monitor controller owns all load pacing.
export function parseRate(value) {
    const text = String(value).trim();
    if (/^(maximum|max)$/i.test(text)) return 0;
    if (!/^\d+$/.test(text) || !Number.isSafeInteger(Number(text)) || Number(text) > 1000000000) {
        throw new Error('Rate must be Maximum or a positive integer up to 1,000,000,000');
    }
    return Number(text);
}
export function readSettings(values) {
    if (values.domain === 'client-movement') {
        const result = { domain: values.domain, max_ms: Number(values.max_ms) };
        if (values.showcase === '1') result.showcase = true;
        const limits = {clients:[1,5], client_spacing:[0,1000000], movement_speed:[0,1000000], input_hz:[1,60], query_hz:[1,60], average_latency_ms:[0,86400000], jitter_ms:[0,Number.MAX_SAFE_INTEGER],
            trail_ms:[200,5000], fade_ms:[100,5000], retention_ms:[1,86400000], checkpoint_interval:[1,1000000000], max_steps:[1,100000]};
        for (const [key,[min,max]] of Object.entries(limits)) {
            const n=Number(values[key] ?? ({client_spacing:16,movement_speed:6})[key]);
            if (!Number.isSafeInteger(n)||n<min||n>max) throw new Error(`Invalid ${key}`);
            result[key]=n;
        }
        if (!Number.isFinite(result.max_ms)||result.max_ms<0.01||result.max_ms>1000) throw new Error('Invalid max_ms');
        if (result.retention_ms < result.average_latency_ms+Math.min(result.average_latency_ms,result.jitter_ms)+result.trail_ms+501) throw new Error('Retention must cover delay + trail + 500 ms');
        return Object.freeze(result);
    }
    const result = { ...values, rate: parseRate(values.rate) };
    const limits = { snapshot_skew: [0, 100], snapshots: [1, 1000000], query_hz: [1, 1000], retention_ms: [1, 86400000],
        checkpoint_interval: [1, 1000000000], max_pending: [1, 10000000], max_targets: [1, 1000000], max_steps: [1, 100000] };
    if (result.rate === 0) {
        delete result.max_pending;
        delete limits.max_pending;
    }
    for (const [key, [min, max]] of Object.entries(limits)) {
        const value = Number(values[key] ?? (key === "snapshot_skew" ? 0 : NaN));
        if (!Number.isSafeInteger(value) || value < min || value > max) throw new Error(`Invalid ${key}`);
        result[key] = value;
    }
    result.max_ms = Number(values.max_ms);
    if (!Number.isFinite(result.max_ms) || result.max_ms < 0.01 || result.max_ms > 1000) throw new Error('Invalid max_ms');
    if (!/^\d+(\.\d{1,3})?$/.test(String(values.average_targets))) throw new Error('Invalid average_targets');
    const average = Number(values.average_targets);
    if (average < 1 || average > result.max_targets || result.max_targets > result.snapshots) {
        throw new Error('Average targets must be between 1 and maximum targets; maximum targets cannot exceed snapshots');
    }
    result.average_targets = String(values.average_targets);
    if (!['counter', 'chain-hash'].includes(result.domain)) throw new Error('Invalid domain');
    return Object.freeze(result);
}

// The launcher supplies startup values through the URL; direct loads use defaults.
export function startupSettings(params) {
    const defaults = {showcase:'0',domain:'counter',snapshots:1000,rate:'Maximum',query_hz:20,retention_ms:10000,
        checkpoint_interval:128,max_pending:20000,average_targets:'1.0',max_targets:2,snapshot_skew:0,
        max_ms:2,max_steps:256,clients:5,client_spacing:16,movement_speed:6,input_hz:1,
        average_latency_ms:1000,jitter_ms:1000,trail_ms:5000,fade_ms:5000};
    return readSettings(Object.fromEntries(Object.entries(defaults).map(([key,value]) => [key,params.get(key) ?? value])));
}
