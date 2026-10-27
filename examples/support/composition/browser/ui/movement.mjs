// Draw only query-owned positions and paths received from Rust.
export function drawMovement(canvas, report) {
    const ctx = canvas.getContext('2d');
    const width = canvas.width = Math.max(320, canvas.clientWidth * (globalThis.devicePixelRatio || 1));
    const height = canvas.height = Math.max(240, width * 0.6);
    const scale = width / 100;
    const point = ([x, y]) => [width / 2 + (x - report.center[0]) * scale, height / 2 - (y - report.center[1]) * scale];
    ctx.fillStyle = '#152535'; ctx.fillRect(0, 0, width, height);
    for (const trail of [...report.trails].reverse()) {
        const [x, y] = point(trail.position);
        ctx.globalAlpha = trail.opacity;
        ctx.fillStyle = trail.corrected ? '#e9aa3a' : trail.current ? '#4bd5dd' : '#7c8b9b';
        ctx.fillRect(x - 1, y - 1, 3, 3);
    }
    ctx.globalAlpha = 1;
    ctx.font = `${Math.max(13, width / 70)}px monospace`;
    for (const client of report.positions) {
        const [x, y] = point(client.position);
        ctx.fillStyle = client.id === 1 ? '#ffda68' : client.corrected ? '#ff8c7d' : '#85da99';
        ctx.fillText(client.id === 1 ? '@' : String.fromCharCode(97 + (client.id - 1) % 26), x, y);
    }
}
