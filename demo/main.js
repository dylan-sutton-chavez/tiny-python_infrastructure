const SZ = 1 << 20;
const WASM_SOURCES = ['./compiler_lib.wasm', 'https://demo.edgepython.com/compiler_lib.wasm'];
const OK = 'ml-auto text-[#7daf7a]';
const ERR = 'ml-auto text-[#d67f6d]';
const MAX_LINES = 99;

const $ = (id) => document.getElementById(id);
const ed = $('ed'), ln = $('ln'), btn = $('run'), term = $('term'), status = $('status');

let wasm = null;

const fmt = (ms) => ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`;

const instantiate = async (url) => {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    try {
        return await WebAssembly.instantiateStreaming(res, {});
    } catch {
        const bytes = await (await fetch(url)).arrayBuffer();
        return WebAssembly.instantiate(bytes, {});
    }
};

const loadWasm = async () => {
    status.textContent = 'loading wasm…';
    const t0 = performance.now();
    try {
        const { instance } = await Promise.any(WASM_SOURCES.map(instantiate));
        wasm = instance.exports;
        btn.disabled = false;
        status.textContent = `ready (${fmt(performance.now() - t0)})`;
        status.className = OK;
    } catch (err) {
        status.textContent = 'load failed';
        status.className = ERR;
        term.textContent = `Could not load wasm.\n\n${err.errors.map(e => e.message).join(' | ')}\n\nFor local development:\n  cd demo/ && make dev`;
    }
};

const runCode = () => {
    if (!wasm) return;
    const srcBytes = new TextEncoder().encode(ed.value);
    if (srcBytes.length > SZ) {
        term.textContent = `error: source exceeds ${SZ} bytes`;
        return;
    }
    const t0 = performance.now();
    new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());
    const outLen = wasm.run(srcBytes.length);
    const out = new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), outLen));
    const elapsed = fmt(performance.now() - t0);
    term.textContent = out;
    status.textContent = `ready (${elapsed})`;
    status.className = OK;
};

const sync = () => {
    const lines = ed.value.split('\n');
    if (lines.length > MAX_LINES) {
        ed.value = lines.slice(0, MAX_LINES).join('\n');
    }
    const n = Math.min(ed.value.split('\n').length, MAX_LINES);
    ln.textContent = Array.from({ length: n }, (_, i) => String(i + 1).padStart(2, '0')).join('\n');
    ln.scrollTop = ed.scrollTop;
};

btn.addEventListener('click', runCode);
ed.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
        e.preventDefault();
        runCode();
    } else if (e.key === 'Enter' && ed.value.split('\n').length >= MAX_LINES) {
        e.preventDefault();
    }
});
ed.oninput = sync;
ed.onscroll = sync;

sync();
loadWasm();