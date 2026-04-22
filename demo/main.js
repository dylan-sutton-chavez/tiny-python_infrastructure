import { CodeJar } from 'https://esm.sh/codejar@4';

// Config

const MAX_LINES = 99;
const TAB_SIZE = 4;
const DEV = !['demo.edgepython.com'].includes(location.hostname);
const FETCH_OPTS = DEV ? { cache: 'no-store' } : undefined;

const DEFAULT_CODE = `"""\nImplements a functional pipeline using function composition and list comprehensions.\nReference: Backus, J. (1978).\n"""\n\ndef double(n: int) -> int:\n    return n * 2\n\ndef square(n: int) -> int:\n    return n * n\n\ndef apply_pipeline(value: int, steps: list) -> int:\n    # Stop recursion when no steps remain\n    if not steps:\n        return value\n\n    first_fn = steps[0]\n    remaining_steps = steps[1:]\n\n    return apply_pipeline(first_fn(value), remaining_steps)\n\ndata: list[int] = [1, 2, 3]\npipeline: list = [double, square]\n\nresult = [apply_pipeline(x, pipeline) for x in data] # Use a list comprehension for efficient data transformation\n\nprint(f"Input: {data}")\nprint(f"Output: {result}")`;

// DOM

const $ = (id) => document.getElementById(id);

const el = {
    ed: $('ed'),
    ln: $('ln'),
    btn: $('run'),
    term: $('term'),
    status: $('status'),
    pkgList: $('pkg-list'),
};

// Utils

const fmt = (ms) => ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`;

const fetchSvg = async (src, attrs = {}) => {
    const text = await fetch(src).then(r => r.text()).catch(() => '');
    const svg = new DOMParser().parseFromString(text, 'image/svg+xml').querySelector('svg');
    if (!svg) return null;
    for (const [k, v] of Object.entries(attrs)) svg.setAttribute(k, v);
    return svg;
};
const loadIcons = async (scope = document) => {
    const nodes = [...scope.querySelectorAll('svg[data-icon]')];
    await Promise.all(nodes.map(async (node) => {
        const src = node.getAttribute('data-icon');
        const attrs = Object.fromEntries(
            [...node.attributes]
                .filter(a => a.name !== 'data-icon')
                .map(a => [a.name, a.value])
        );
        const svg = await fetchSvg(src, attrs);
        if (svg) node.replaceWith(svg);
    }));
};

// Status

const Status = (() => {
    const CLS = { ok: 'ml-auto text-[#7daf7a]', err: 'ml-auto text-[#d67f6d]' };
    return {
        ok: (text) => { el.status.textContent = text; el.status.className = CLS.ok;  },
        err: (text) => { el.status.textContent = text; el.status.className = CLS.err; },
    };
})();

// Highlighter

const Highlighter = (() => {
    const KW = new Set([
        'as','if','in','is','or',
        'and','def','del','for','not','try',
        'case','elif','else','from','pass','type','with',
        'async','await','break','class','match','raise','while','yield',
        'assert','except','global','import','lambda','return',
        'finally','continue','nonlocal',
    ]);
    const BI = new Set([
        'print','len','range','int','str','float','list','dict','tuple','set',
        'bool','isinstance','issubclass','enumerate','zip','map','filter',
        'abs','min','max','sum','round','pow','divmod','hash','id','repr',
        'ord','chr','hex','oct','bin','open','input','iter','next','reversed','sorted',
        'any','all','format','frozenset','bytearray','bytes','complex','memoryview',
        'object','property','staticmethod','classmethod','super','slice',
        'callable','getattr','setattr','hasattr','delattr','dir','vars','globals','locals',
        'NotImplemented','Ellipsis','self','cls',
    ]);
    const LIT = new Set(['True', 'False', 'None']);

    const WORD_CLS = [[KW, 'tk-kw'], [LIT, 'tk-lit'], [BI, 'tk-bi']];
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

    return {
        highlight: (src) => esc(src).replace(TOKEN_RE, tokenize),
    };
})();

// Worker

const PythonWorker = (() => {
    const worker = new Worker('./worker.js');

    const resolveWasmUrl = async () => {
        const ver = await fetch('./version.json', { cache: 'no-store' }).then(r => r.ok ? r.json() : {}).catch(() => ({}));
        const bust = ver.v ? `?v=${ver.v}` : '';
        return DEV
            ? `https://demo.edgepython.com/compiler_lib.wasm${bust}`
            : `./compiler_lib.wasm${bust}`;
    };

    worker.onmessage = ({ data }) => {
        if (data.type === 'ready') {
            el.btn.disabled = false;
            Status.ok(`Ready${DEV ? ' - Dev' : ''} (Loaded in ${fmt(data.ms)})`);
        } else if (data.type === 'result') {
            el.term.textContent = data.out;
            Status.ok(`Ran in ${fmt(data.ms)}`);
            el.btn.disabled = false;
        } else if (data.type === 'error') {
            Status.err('Load failed');
            el.term.textContent = `Could not load WASM.\n\n${data.message}`;
        }
    };

    return {
        load: async () => {
            Status.ok('Loading WASM...');
            const url = await resolveWasmUrl();
            worker.postMessage({ type: 'load', url, opts: FETCH_OPTS ?? {} });
        },
        run: (src) => {
            Status.ok('Running...');
            el.btn.disabled = true;
            worker.postMessage({ type: 'run', src });
        },
    };
})();

// Editor

const Editor = (() => {
    const jar = CodeJar(el.ed, (editor) => {
        editor.innerHTML = Highlighter.highlight(editor.textContent);
    }, {
        tab: '    ',
        indentOn: /[:\[({][ \t]*$/,
        spellcheck: false,
        addClosing: true,
    });

    const syncLineNumbers = () => {
        const lines = jar.toString().replace(/\n$/, '').split('\n');
        const n = Math.max(1, Math.min(lines.length, MAX_LINES));
        el.ln.textContent = Array.from({ length: n }, (_, i) => String(i + 1).padStart(2, '00')).join('\n');
        el.ln.scrollTop = el.ed.scrollTop;
    };

    const handleBackspace = (e) => {
        const pos = jar.save();
        if (pos.start !== pos.end) return;
        const caret = pos.start;
        if (caret === 0) return;
        const text = jar.toString();
        const lineStart = text.lastIndexOf('\n', caret - 1) + 1;
        const before = text.slice(lineStart, caret);
        if (!before.length || !/^[ \t]+$/.test(before)) return;
        e.preventDefault();
        e.stopPropagation();
        const prevStop = Math.floor((before.length - 1) / TAB_SIZE) * TAB_SIZE;
        jar.restore({ start: caret - (before.length - prevStop), end: caret });
        document.execCommand('delete');
    };

    el.ed.addEventListener('keydown', (e) => {
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            e.preventDefault();
            e.stopPropagation();
            PythonWorker.run(jar.toString());
            return;
        }
        if (e.key === 'Enter' && jar.toString().split('\n').length >= MAX_LINES) {
            e.preventDefault();
            e.stopPropagation();
            return;
        }
        if (e.key === 'Backspace') handleBackspace(e);
    }, true);

    el.ed.addEventListener('scroll', () => { el.ln.scrollTop = el.ed.scrollTop; });

    jar.onUpdate(syncLineNumbers);
    jar.updateCode(DEFAULT_CODE);

    return {
        getCode: () => jar.toString(),
    };
})();

// Packages

const Packages = (() => {
    const createItem = async (pkg) => {
        const iconSvg = await fetchSvg(pkg.icon, { class: 'size-3.5', 'aria-hidden': 'true', focusable: 'false' });

        const li = document.createElement('li');
        li.className = 'bg-[#1c1c1c] border border-[#2d2d2d] rounded-md w-full';
        li.innerHTML = `
            <div class="flex items-center">
                <div class="px-2 py-2.5">
                    <h3 class="text-[12px]">${pkg.name}</h3>
                </div>
                <div class="flex items-center gap-2 ml-auto px-1.5">
                    ${iconSvg?.outerHTML ?? ''}
                    <button disabled aria-label="Reload ${pkg['aria-label']}" class="bg-[#ffffff] p-[3.7px] rounded-full opacity-40 cursor-not-allowed">
                        <svg data-icon="./static/cloud-download.svg" class="size-3.5" aria-hidden="true" focusable="false"></svg>
                    </button>
                </div>
            </div>`;
        return li;
    };

    return {
        load: async () => {
            if (!el.pkgList) return;
            const packages = await fetch('./packages.json').then(r => r.json()).catch(() => []);
            const items = await Promise.all(packages.map(createItem));
            items.forEach(item => el.pkgList.appendChild(item));
            await loadIcons(el.pkgList);
        },
    };
})();

// Init

el.btn.addEventListener('click', () => PythonWorker.run(Editor.getCode()));

loadIcons();
PythonWorker.load();
Packages.load();