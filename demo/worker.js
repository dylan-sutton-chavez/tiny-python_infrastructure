const SZ = 1 << 20;
let wasm;

self.onmessage = async ({ data }) => {
    if (data.type === 'load') {
        try {
            const res = await fetch(data.url, data.opts);
            const t0 = performance.now();
            const { instance } = await WebAssembly.instantiateStreaming(res, {});
            wasm = instance.exports;
            const ms = performance.now() - t0;
            self.postMessage({ type: 'ready', ms });
        } catch (err) {
            self.postMessage({ type: 'error', message: err.message });
        }

    } else if (data.type === 'run') {
        const srcBytes = new TextEncoder().encode(data.src);
        if (srcBytes.length > SZ) {
            self.postMessage({ type: 'result', out: `Error: Source exceeds ${SZ} bytes` });
            return;
        }
        new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());
        const t0 = performance.now();
        const len = wasm.run(srcBytes.length);
        const ms = performance.now() - t0;
        const out = new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len));
        self.postMessage({ type: 'result', out, ms });
    }
};