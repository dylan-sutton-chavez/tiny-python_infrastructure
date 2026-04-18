import { CodeJar } from 'https://esm.sh/codejar@4';

const SZ = 1 << 20;
const DEV = !['demo.edgepython.com'].includes(location.hostname);
const WASM_SOURCES = DEV
    ? ['https://demo.edgepython.com/compiler_lib.wasm']
    : ['./compiler_lib.wasm'];
const FETCH_OPTS = DEV ? { cache: 'no-store' } : undefined;

const DEFAULT_CODE = `def add(a: int, b: int) -> int:\n    return a + b\nresult: int = add(13, 20)\nprint(result)`;

const CLS = { ok: 'ml-auto text-[#7daf7a]', err: 'ml-auto text-[#d67f6d]' };
const MAX_LINES = 99;
const $ = (id) => document.getElementById(id);
const [ed, ln, btn, term, statusEl] = ['ed', 'ln', 'run', 'term', 'status'].map($);

const KW = new Set([
    'as','if','in','is','or',
    'and','def','del','for','not','try',
    'case','elif','else','from','pass','type','with',
    'async','await','break','class','match','raise','while','yield',
    'assert','except','global','import','lambda','return',
    'finally',
    'continue','nonlocal'
]);
const BI = new Set([
    'print','len','range','int','str','float','list','dict','tuple','set',
    'bool','isinstance','issubclass','enumerate','zip','map','filter',
    'abs','min','max','sum','round','pow','divmod','hash','id','repr',
    'ord','chr','hex','oct','bin','open','input','iter','next','reversed','sorted',
    'any','all','format','frozenset','bytearray','bytes','complex','memoryview',
    'object','property','staticmethod','classmethod','super','slice',
    'callable','getattr','setattr','hasattr','delattr','dir','vars','globals','locals',
    'NotImplemented','Ellipsis','self','cls'
]);
const LIT = new Set(['True','False','None']);

const TOKEN_RE = /(#[^\n]*)|((?:\b[fFrRbBuU]{1,2})?(?:"""[\s\S]*?"""|'''[\s\S]*?'''|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'))|(0[xX][\da-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|\d[\d_]*(?:\.[\d_]*)?(?:[eE][+-]?\d+)?[jJ]?|\.\d[\d_]*(?:[eE][+-]?\d+)?[jJ]?)|([A-Za-z_]\w*)/g;

const esc = (s) => s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));

const highlight = (src) => {
    const escaped = esc(src);
    return escaped.replace(TOKEN_RE, (m, com, str, num, word, offset, fullStr) => {
        if (com)  return `<span class="tk-com">${com}</span>`;
        if (str)  return `<span class="tk-str">${str}</span>`;
        if (num)  return `<span class="tk-num">${num}</span>`;
        if (word) {        
            const isEntity = fullStr[offset - 1] === '&' && fullStr[offset + word.length] === ';';
            if (isEntity) return word;

            if (KW.has(word))  return `<span class="tk-kw">${word}</span>`;
            if (LIT.has(word)) return `<span class="tk-lit">${word}</span>`;
            if (BI.has(word))  return `<span class="tk-bi">${word}</span>`;

            const isFunction = /^\s*\(/.test(fullStr.slice(offset + word.length));
            if (isFunction) {
                return `<span class="tk-func">${word}</span>`;
            } else {
                return `<span class="tk-var">${word}</span>`;
            }
        }
        return m;
    });
};
const jar = CodeJar(ed, (editor) => {
    editor.innerHTML = highlight(editor.textContent);
}, {
    tab: '    ',
    indentOn: /:\s*$/,
    spellcheck: false,
    addClosing: false,
});

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
    setStatus('Loading WASM...', CLS.ok);
    try {
        const [{ instance }, t] = await time(() => Promise.any(WASM_SOURCES.map(instantiate)));
        wasm = instance.exports;
        btn.disabled = false;
        setStatus(`Ready (${t}${DEV ? ' · Dev' : ''})`);
    } catch (err) {
        setStatus('Load failed', CLS.err);
        term.textContent = `Could not load WASM.\n\n${err.errors.map(e => e.message).join(' | ')}`;
    }
};

const runCode = async () => {
    if (!wasm) return;
    const srcBytes = new TextEncoder().encode(jar.toString());
    if (srcBytes.length > SZ) return void (term.textContent = `Error: Source exceeds ${SZ} bytes`);

    setStatus('Running...', CLS.ok);
    btn.disabled = true;

    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));

    const [out, t] = await time(() => {
        new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());
        const len = wasm.run(srcBytes.length);
        return new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len));
    });

    term.textContent = out;
    setStatus(`Ready (${t})`);
    btn.disabled = false;
};

const sync = () => {
    const text = jar.toString().replace(/\n$/, '');
    const n = Math.max(1, Math.min(text.split('\n').length, MAX_LINES));
    ln.textContent = Array.from({ length: n }, (_, i) => String(i + 1).padStart(2, '0')).join('\n');
    ln.scrollTop = ed.scrollTop;
};

btn.addEventListener('click', runCode);

ed.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
        e.preventDefault();
        e.stopPropagation();
        runCode();
        return;
    }
    if (e.key === 'Enter' && jar.toString().split('\n').length >= MAX_LINES) {
        e.preventDefault();
        e.stopPropagation();
        return;
    }
    if (e.key === 'Backspace') {
        const pos = jar.save();
        if (pos.start !== pos.end) return;
        const caret = pos.start;
        if (caret === 0) return;
        const text = jar.toString();
        const lineStart = text.lastIndexOf('\n', caret - 1) + 1;
        const before = text.slice(lineStart, caret);
        if (before.length === 0 || !/^[ \t]+$/.test(before)) return;
        e.preventDefault();
        e.stopPropagation();
        const TAB = 4;
        const prevStop = Math.floor((before.length - 1) / TAB) * TAB;
        const del = before.length - prevStop;
        jar.restore({ start: caret - del, end: caret });
        document.execCommand('delete');
    }
}, true);

ed.addEventListener('scroll', () => { ln.scrollTop = ed.scrollTop; });

jar.onUpdate(sync);
jar.updateCode(DEFAULT_CODE);

loadWasm();