import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

const wasi = new WASI({ version: 'preview1', returnOnExit: true, args: ['contime-wasm-example', ...process.argv.slice(2)] });
const bytes = await readFile(new URL('./target/wasm32-wasip1/release/contime-wasm-example.wasm', import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, wasi.getImportObject());
process.exitCode = wasi.start(instance);
