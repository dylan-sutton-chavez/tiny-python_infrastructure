import { CodeJar } from 'https://esm.sh/codejar@4';

const SZ = 1 << 20;
const DEV = !['demo.edgepython.com'].includes(location.hostname);
const FETCH_OPTS = DEV ? { cache: 'no-store' } : undefined;

const _ver = await fetch('./version.json', { cache: 'no-store' })
    .then(r => r.ok ? r.json() : {})
    .catch(() => ({}));
const _bust = _ver.v ? `?v=${_ver.v}` : '';

const WASM_SOURCES = DEV
    ? [`https://demo.edgepython.com/compiler_lib.wasm${_bust}`]
    : [`./compiler_lib.wasm${_bust}`];

const DEFAULT_CODE = `"""\nImplements a functional pipeline using function composition and list comprehensions.\nReferences: Backus, J. (1978).\n"""\n\ndef double(n: int) -> int:\n    return n * 2\n\ndef square(n: int) -> int:\n    return n * n\n\ndef apply_pipeline(value: int, steps: list) -> int:\n    # Stop recursion when no steps remain\n    if not steps:\n        return value\n\n    first_fn = steps[0]\n    remaining_steps = steps[1:]\n\n    return apply_pipeline(first_fn(value), remaining_steps)\n\ndata: list[int] = [1, 2, 3]\npipeline: list = [double, square]\n\nresult = [apply_pipeline(x, pipeline) for x in data] # Use a list comprehension for efficient data transformation\n\nprint(f"Input: {data}")\nprint(f"Output: {result}")`;

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

const WORD_CLS = [
    [KW, 'tk-kw'],
    [LIT, 'tk-lit'],
    [BI, 'tk-bi'],
];

const TOKEN_RE = /(#[^\n]*)|((?:\b[fFrRbBuU]{1,2})?(?:"""[\s\S]*?"""|'''[\s\S]*?'''|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'))|(0[xX][\da-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|\d[\d_]*(?:\.[\d_]*)?(?:[eE][+-]?\d+)?[jJ]?|\.\d[\d_]*(?:[eE][+-]?\d+)?[jJ]?)|([A-Za-z_]\w*)/g;

const esc = (s) => s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
const span = (cls, s) => `<span class="${cls}">${s}</span>`;

const tokenize = (m, com, str, num, word, offset, fullStr) => {
    if (com) return span('tk-com', com);
    if (str) {
        if (/^[fFrRbBuU]*[fF]/i.test(str)) {
            const body = str.replace(/\{\{|\}\}|\{([^{}]*)\}/g, (m, expr) =>
                expr != null
                    ? `{${expr.replace(new RegExp(TOKEN_RE.source, TOKEN_RE.flags), tokenize)}}`
                    : m
            );
            return span('tk-str', body);
        }
        return span('tk-str', str);
    }
    if (num) return span('tk-num', num);
    if (word) {
        if (fullStr[offset - 1] === '&' && fullStr[offset + word.length] === ';') return word;
        for (const [set, cls] of WORD_CLS) if (set.has(word)) return span(cls, word);
        return span(/^\s*\(/.test(fullStr.slice(offset + word.length)) ? 'tk-func' : 'tk-var', word);
    }
    return m;
};

const highlight = (src) => esc(src).replace(TOKEN_RE, tokenize);

const jar = CodeJar(ed, (editor) => {
    editor.innerHTML = highlight(editor.textContent);
}, {
    tab: '    ',
    indentOn: /:[ \t]*$/,
    spellcheck: false,
    addClosing: false,
});

const fmt = (ms) => ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`;
const setStatus = (text, cls = CLS.ok) => (statusEl.textContent = text, statusEl.className = cls);

const worker = new Worker('./worker.js');

const loadWasm = () => {
    setStatus('Loading WASM...', CLS.ok);
    worker.postMessage({
        type: 'load',
        url: WASM_SOURCES[0],
        opts: FETCH_OPTS ?? {},
    });
};

const runCode = () => {
    setStatus('Running...', CLS.ok);
    btn.disabled = true;
    worker.postMessage({ type: 'run', src: jar.toString() });
};

worker.onmessage = ({ data }) => {
    if (data.type === 'ready') {
        btn.disabled = false;
        setStatus(`Ready${DEV ? ' - Dev' : ''} (Loaded in ${fmt(data.ms)})`);
    } else if (data.type === 'result') {
        term.textContent = data.out;
        setStatus(`Ran in ${fmt(data.ms)}`);
        btn.disabled = false;
    } else if (data.type === 'error') {
        setStatus('Load failed', CLS.err);
        term.textContent = `Could not load WASM.\n\n${data.message}`;
    }
};

const sync = () => {
    const text = jar.toString().replace(/\n$/, '');
    const n = Math.max(1, Math.min(text.split('\n').length, MAX_LINES));
    ln.textContent = Array.from({ length: n }, (_, i) => String(i + 1).padStart(2, '00')).join('\n');
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

const RELOAD_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#1c1c1c" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3.5" aria-hidden="true" focusable="false"><path d="M12 13v8l-4-4"/><path d="m12 21 4-4"/><path d="M4.393 15.269A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.436 8.284"/></svg>`;

const fetchSvg = async (src) => {
    const text = await fetch(src).then(r => r.text()).catch(() => '');
    const svg = new DOMParser().parseFromString(text, 'image/svg+xml').querySelector('svg');
    if (!svg) return '';
    svg.setAttribute('class', 'size-3.5');
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('focusable', 'false');
    return svg.outerHTML;
};

const loadPackages = async () => {
    const list = $('pkg-list');
    if (!list) return;

    const packages = await fetch('./packages.json').then(r => r.json()).catch(() => []);

    for (const pkg of packages) {
        const iconSvg = await fetchSvg(pkg.icon);
        const li = document.createElement('li');
        li.className = 'bg-[#1c1c1c] border border-[#2d2d2d] rounded-md w-full';
        li.innerHTML = `
            <div class="flex items-center">
                <div class="px-2 py-2.5">
                    <h3 class="text-[12px]">${pkg.name}</h3>
                </div>
                <div class="flex items-center gap-2 ml-auto px-1.5">
                    ${iconSvg}
                    <button disabled aria-label="Reload ${pkg['aria-label']}" class="bg-[#ffffff] p-[3.7px] rounded-full opacity-40 cursor-not-allowed">
                        ${RELOAD_SVG}
                    </button>
                </div>
            </div>`;
        list.appendChild(li);
    }
};

ed.addEventListener('scroll', () => { ln.scrollTop = ed.scrollTop; });

jar.onUpdate(sync);
jar.updateCode(DEFAULT_CODE);

loadWasm();
loadPackages();