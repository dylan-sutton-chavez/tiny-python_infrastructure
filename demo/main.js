const SZ = 1 << 20;
const DEV = !['demo.edgepython.com'].includes(location.hostname);
const WASM_SOURCES = DEV
    ? ['https://demo.edgepython.com/compiler_lib.wasm']
    : ['./compiler_lib.wasm'];
const FETCH_OPTS = DEV ? { cache: 'no-store' } : undefined;

const CLS = { ok: 'ml-auto text-[#7daf7a]', err: 'ml-auto text-[#d67f6d]' };
const MAX_LINES = 99;
const $ = (id) => document.getElementById(id);
const [ed, ln, btn, term, statusEl] = ['ed', 'ln', 'run', 'term', 'status'].map($);

let wasm;

const fmt = (ms) => ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`;
const setStatus = (text, cls = CLS.ok) => (statusEl.textContent = text, statusEl.className = cls);
const time = async (fn) => { const t0 = performance.now(); const r = await fn(); return [r, fmt(performance.now() - t0)]; };

const instantiate = async (url) => {
    const res = await fetch(url, FETCH_OPTS);
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    try {
        return await WebAssembly.instantiateStreaming(res, {});
    } catch {
        return WebAssembly.instantiate(await (await fetch(url, FETCH_OPTS)).arrayBuffer(), {});
    }
};

const loadWasm = async () => {
    setStatus('loading wasm…', CLS.ok);
    try {
        const [{ instance }, t] = await time(() => Promise.any(WASM_SOURCES.map(instantiate)));
        wasm = instance.exports;
        btn.disabled = false;
        setStatus(`ready (${t}${DEV ? ' · dev' : ''})`);
    } catch (err) {
        setStatus('load failed', CLS.err);
        term.textContent = `Could not load wasm.\n\n${err.errors.map(e => e.message).join(' | ')}`;
    }
};

const runCode = async () => {
    if (!wasm) return;
    const srcBytes = new TextEncoder().encode(ed.value);
    if (srcBytes.length > SZ) return void (term.textContent = `error: source exceeds ${SZ} bytes`);
    const [out, t] = await time(() => {
        new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());
        const len = wasm.run(srcBytes.length);
        return new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len));
    });
    term.textContent = out;
    setStatus(`ready (${t})`);
};

const sync = () => {
    const lines = ed.value.split('\n');
    if (lines.length > MAX_LINES) ed.value = lines.slice(0, MAX_LINES).join('\n');
    const n = Math.min(ed.value.split('\n').length, MAX_LINES);
    ln.textContent = Array.from({ length: n }, (_, i) => String(i + 1).padStart(2, '0')).join('\n');
    ln.scrollTop = ed.scrollTop;
};

btn.addEventListener('click', runCode);
ed.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); runCode(); }
    else if (e.key === 'Enter' && ed.value.split('\n').length >= MAX_LINES) e.preventDefault();
});
ed.oninput = ed.onscroll = sync;

sync();
loadWasm();