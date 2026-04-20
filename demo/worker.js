const SZ = 1 << 20;
let wasmModule = null;

self.onmessage = async ({ data }) => {

    if (data.type === 'load') {
        try {
            const t0 = performance.now();

            wasmModule = await WebAssembly.compileStreaming(
                fetch(data.url, data.opts)
            );

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

        const { exports: wasm } = await WebAssembly.instantiate(wasmModule);

        new Uint8Array(wasm.memory.buffer).set(srcBytes, wasm.src_ptr());

        const t0 = performance.now();
        const len = wasm.run(srcBytes.length);
        const ms = performance.now() - t0;

        const out = new TextDecoder().decode(
            new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len)
        );

        // la instance anterior es GC'd automáticamente por el browser
        self.postMessage({ type: 'result', out, ms });
    }
};