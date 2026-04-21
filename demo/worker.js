const SZ = 1 << 20;

let wasmModule = null;

const handlers = {
    load: async ({ url, opts }) => {
        try {
            const t0 = performance.now();
            wasmModule = await WebAssembly.compileStreaming(fetch(url, opts));
            self.postMessage({ type: 'ready', ms: performance.now() - t0 });
        } catch (err) {
            self.postMessage({ type: 'error', message: err.message });
        }
    },

    run: async ({ src }) => {
        const srcBytes = new TextEncoder().encode(src);

        if (srcBytes.length > SZ) {
            self.postMessage({ type: 'result', out: `Error: Source exceeds ${SZ} bytes` });
            return;
        }

        const { exports: wasm } = await WebAssembly.instantiate(wasmModule);
        new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());

        const t0 = performance.now();
        const len = wasm.run(srcBytes.length);
        const ms = performance.now() - t0;

        const out = new TextDecoder().decode(
            new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len)
        );

        self.postMessage({ type: 'result', out, ms });
    },
};

self.onmessage = ({ data }) => handlers[data.type]?.(data);