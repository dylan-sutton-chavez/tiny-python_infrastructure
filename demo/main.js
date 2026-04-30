import { CodeJar } from 'https://esm.sh/codejar@4';

// Config

const MAX_LINES = 99;
const TAB_SIZE = 4;
const DEV = !['demo.edgepython.com'].includes(location.hostname);
const FETCH_OPTS = DEV ? { cache: 'no-store' } : undefined;

const DEFAULT_CODE = `"""\nFunctional pipeline using lambdas, closures and list comprehensions.\nReference: Backus, J. (1978).\n"""\n\ndouble = lambda n: n * 2\nsquare = lambda n: n * n\n\ndef compose(*fns):\n    def piped(x):\n        for f in fns:\n            x = f(x)\n        return x\n    return piped\n\npipeline = compose(double, square)\n\ndata = [1, 2, 3]\nresult = [pipeline(x) for x in data]\n\nprint(f"Input:  {data}")\nprint(f"Output: {result}")`;

// DOM

const $ = (id) => document.getElementById(id);
const el = {
    ed: $('ed'), ln: $('ln'), btn: $('run'),
    term: $('term'), status: $('status'),
};

// Utils

const fmt = (ms) => ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`;

const fetchSvg = async (src, attrs = {}) => {
    const text = await fetch(src).then(r => r.text()).catch(() => '');
    const svg = new DOMParser().parseFromString(text, 'image/svg+xml').querySelector('svg');
    if (svg) for (const [k, v] of Object.entries(attrs)) svg.setAttribute(k, v);
    return svg;
};

const loadIcons = (scope = document) => Promise.all(
    [...scope.querySelectorAll('svg[data-icon]')].map(async (node) => {
        const attrs = Object.fromEntries(
            [...node.attributes].filter(a => a.name !== 'data-icon').map(a => [a.name, a.value])
        );
        const svg = await fetchSvg(node.getAttribute('data-icon'), attrs);
        if (svg) node.replaceWith(svg);
    })
);

const setStatus = (text, cls) => {
    el.status.textContent = text;
    el.status.className = `ml-auto ${cls}`;
};
const ok = (t) => setStatus(t, 'text-[#7daf7a]');
const err = (t) => setStatus(t, 'text-[#d67f6d]');

// Highlighter

const Highlighter = (() => {
    const KW = new Set(['as','if','in','is','or','and','def','del','for','not','try','case','elif','else','from','pass','type','with','async','await','break','class','match','raise','while','yield','assert','except','global','import','lambda','return','finally','continue','nonlocal']);
    const BI = new Set(['print','len','range','int','str','float','list','dict','tuple','set','bool','isinstance','enumerate','zip','abs','min','max','sum','round','pow','divmod','hash','id','repr','ord','chr','hex','oct','bin','input','reversed','sorted','any','all','format','ascii','callable','getattr','hasattr','type','self','cls']);
    const LIT = new Set(['True', 'False', 'None']);
    const CLASSES = [[KW, 'tk-kw'], [LIT, 'tk-lit'], [BI, 'tk-bi']];

    const TOKEN_RE = /(#[^\n]*)|((?:\b[fFrRbBuU]{1,2})?(?:"""[\s\S]*?"""|'''[\s\S]*?'''|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'))|(0[xX][\da-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|\d[\d_]*(?:\.[\d_]*)?(?:[eE][+-]?\d+)?[jJ]?|\.\d[\d_]*(?:[eE][+-]?\d+)?[jJ]?)|([A-Za-z_]\w*)/g;
    const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;' };
    const esc = (s) => s.replace(/[&<>]/g, (c) => ESC[c]);
    const span = (cls, s) => `<span class="${cls}">${s}</span>`;

    const tokenize = (m, com, str, num, word, offset, full) => {
        if (com) return span('tk-com', com);
        if (str) {
            if (/^[fFrRbBuU]*[fF]/.test(str)) {
                return span('tk-str', str.replace(/\{\{|\}\}|\{([^{}]*)\}/g, (m2, expr) =>
                    expr != null ? `{${expr.replace(new RegExp(TOKEN_RE.source, TOKEN_RE.flags), tokenize)}}` : m2
                ));
            }
            return span('tk-str', str);
        }
        if (num) return span('tk-num', num);
        if (word) {
            if (full[offset - 1] === '&' && full[offset + word.length] === ';') return word;
            for (const [set, cls] of CLASSES) if (set.has(word)) return span(cls, word);
            return span(/^\s*\(/.test(full.slice(offset + word.length)) ? 'tk-func' : 'tk-var', word);
        }
        return m;
    };

    return { highlight: (src) => esc(src).replace(TOKEN_RE, tokenize) };
})();

// Worker

const PythonWorker = (() => {
    const worker = new Worker('./worker.js');

    const resolveUrl = async () => {
        const ver = await fetch('./version.json', { cache: 'no-store' }).then(r => r.ok ? r.json() : {}).catch(() => ({}));
        const bust = ver.v ? `?v=${ver.v}` : '';
        return DEV ? `https://demo.edgepython.com/compiler_lib.wasm${bust}` : `./compiler_lib.wasm${bust}`;
    };

    const onMsg = {
        ready: ({ ms }) => { el.btn.disabled = false; ok(`Ready${DEV ? ' - Dev' : ''} (Loaded in ${fmt(ms)})`); },
        result: ({ out, ms }) => { el.term.textContent = out; ok(`Ran in ${fmt(ms)}`); el.btn.disabled = false; },
        error: ({ message }) => { err('Load failed'); el.term.textContent = `Could not load WASM.\n\n${message}`; },
    };
    worker.onmessage = ({ data }) => onMsg[data.type]?.(data);

    return {
        load: async () => {
            ok('Loading WASM...');
            worker.postMessage({ type: 'load', url: await resolveUrl(), opts: FETCH_OPTS ?? {} });
        },
        run: (src) => {
            ok('Running...');
            el.btn.disabled = true;
            worker.postMessage({ type: 'run', src });
        },
    };
})();

// Pure transitions avoid execCommand, resolving auto-close degradation and the bug where backspace eats lines.

const Editor = (() => {
    const PAIRS = { '(': ')', '[': ']', '{': '}', '"': '"', "'": "'" };
    const OPENERS = new Set(Object.keys(PAIRS));
    const CLOSERS = new Set(Object.values(PAIRS));
    const STRING_START = /^([fFrRbBuU]{0,2})("""|'''|"|')/;

    // Tiny state machine: walk from 0 to caret toggling between code/string; `quote === ''` means we're in code; otherwise we're inside that string.
    const stringCtx = (src, caret) => {
        let i = 0, quote = '', isF = false;
        while (i < caret) {
            if (!quote) {
                if (src[i] === '#') {
                    const nl = src.indexOf('\n', i);
                    if (nl === -1 || nl >= caret) return { inStr: false, isF: false };
                    i = nl + 1; continue;
                }
                const m = src.slice(i).match(STRING_START);
                if (m && i + m[0].length <= caret) {
                    quote = m[2]; isF = /[fF]/.test(m[1]);
                    i += m[0].length; continue;
                }
                i++;
            } else {
                if (quote.length === 1 && src[i] === '\\') { i += 2; continue; }
                if (src.slice(i, i + quote.length) === quote) {
                    i += quote.length; quote = ''; isF = false; continue;
                }
                i++;
            }
        }
        return { inStr: !!quote, isF };
    };

    const transitions = {
        // Typed character: skip existing closer, or auto-close an opener.
        char: (text, caret, key) => {
            if (CLOSERS.has(key) && text[caret] === key) {
                return { text, caret: caret + 1 };
            }
            if (OPENERS.has(key)) {
                const { inStr, isF } = stringCtx(text, caret);
                // Inside a normal string we don't auto-close anything; inside an f-string we still want `{` -> `{}` for expressions.
                if (inStr && !(isF && key === '{')) return null;
                return {
                    text: text.slice(0, caret) + key + PAIRS[key] + text.slice(caret),
                    caret: caret + 1,
                };
            }
            return null;
        },

        // Backspace: delete empty pair, or snap indent to previous tab stop.
        backspace: (text, caret) => {
            if (caret === 0) return null;
            if (PAIRS[text[caret - 1]] === text[caret]) {
                return {
                    text: text.slice(0, caret - 1) + text.slice(caret + 1),
                    caret: caret - 1,
                };
            }
            const lineStart = text.lastIndexOf('\n', caret - 1) + 1;
            const before = text.slice(lineStart, caret);
            // contenteditable substitutes regular spaces with NBSP (U+00A0) on
            // auto-indented lines after Enter. Without contemplating it the indent
            // snap fails and backspace deletes one byte at a time.
            if (!before.length || !/^[ \t\u00a0]+$/.test(before)) return null;
            const prevStop = Math.floor((before.length - 1) / TAB_SIZE) * TAB_SIZE;
            const del = before.length - prevStop;
            return { text: text.slice(0, caret - del) + text.slice(caret), caret: caret - del };
        },
    };

    const jar = CodeJar(el.ed,
        (ed) => { ed.innerHTML = Highlighter.highlight(ed.textContent); },
        { tab: ' '.repeat(TAB_SIZE), indentOn: /[:\[({][ \t]*$/, spellcheck: false, addClosing: false }
    );

    const apply = (result) => {
        if (!result) return false;
        jar.updateCode(result.text);
        jar.restore({ start: result.caret, end: result.caret });
        return true;
    };

    const syncLines = () => {
        const lines = jar.toString().replace(/\n$/, '').split('\n');
        const n = Math.max(1, Math.min(lines.length, MAX_LINES));
        el.ln.textContent = Array.from({ length: n }, (_, i) => String(i + 1).padStart(2, '0')).join('\n');
        el.ln.scrollTop = el.ed.scrollTop;
    };

    el.ed.addEventListener('keydown', (e) => {
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            e.preventDefault(); PythonWorker.run(jar.toString()); return;
        }
        if (e.key === 'Enter' && jar.toString().split('\n').length >= MAX_LINES) {
            e.preventDefault(); return;
        }
        const pos = jar.save();
        if (pos.start !== pos.end) return;

        const text = jar.toString();
        const result =
            e.key === 'Backspace' ? transitions.backspace(text, pos.start) :
            (OPENERS.has(e.key) || CLOSERS.has(e.key)) ? transitions.char(text, pos.start, e.key) :
            null;

        if (apply(result)) { e.preventDefault(); e.stopPropagation(); }
    }, true);

    el.ed.addEventListener('scroll', () => { el.ln.scrollTop = el.ed.scrollTop; });
    jar.onUpdate(syncLines);
    jar.updateCode(DEFAULT_CODE);

    return { getCode: () => jar.toString() };
})();

// Init

el.btn.addEventListener('click', () => PythonWorker.run(Editor.getCode()));
loadIcons();
PythonWorker.load();