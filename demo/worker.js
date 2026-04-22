const source_size = 1 << 20; // 1 MiB limit.

let wasmModule = null;

const handlers = {
    load: async ({ url, opts }) => {
        try {
            const t0 = performance.now();
            // Compile without instantiating to allow multiple runs from the same module.
            wasmModule = await WebAssembly.compileStreaming(fetch(url, opts));
            self.postMessage({ type: 'ready', ms: performance.now() - t0 });
        } catch (err) {
            self.postMessage({ type: 'error', message: err.message });
        }
    },

    run: async ({ src }) => {
        const srcBytes = new TextEncoder().encode(src);

        if (srcBytes.length > source_size) {
            self.postMessage({ type: 'result', out: `Error: Source exceeds ${source_size} bytes` });
            return;
        }

        const { exports: wasm } = await WebAssembly.instantiate(wasmModule);

        // Copy source bytes into WASM linear memory.
        new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());

        const t0 = performance.now();
        const len = wasm.run(srcBytes.length);
        const ms = performance.now() - t0;

        // Extract output from WASM memory using returned length and pointer
        const out = new TextDecoder().decode(
            new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len)
        );

        self.postMessage({ type: 'result', out, ms });
    },
};

self.onmessage = ({ data }) => handlers[data.type]?.(data);