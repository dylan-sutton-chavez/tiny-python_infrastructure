## Edge Python Demo

Run Edge Python (based on CPython 3.13) directly in the browser. Edge Python is a single-pass SSA compiler with adaptive inline caching and template memoization, written in Rust and compiled to WebAssembly.

* **Demo:** *[demo.edgepython.com](https://demo.edgepython.com/)*
* **Docs:** *[edgepython.com](https://demo.edgepython.com/)*

---

## Features

* **In-Browser Execution:** Fully client-side using WebAssembly and Web Workers.
* **Lightweight Code Editor:** Custom syntax highlighting, line numbering, and auto-indentation using CodeJar.
* **Underweight:** Maintaining a release size of approximately 100KB (including HTML, WASM, JS, and TailwindCSS).
* **High Disponibility:** Maintained a Total Interactive Time (TTI) of 514ms, based on Cloudflare performance tests.

## Local Start

Since the application fetches WebAssembly modules and uses Web Workers, you need to serve it through a local web server (opening `index.html` directly via `file://` will cause CORS/fetch errors). 

You can quickly start a local server using Python: `bash python -m http.server 8000` Then, open http://localhost:8000 in your web browser.

> The latest active WASM release from GitHub will be used to decouple frontend and backend development.

### Project Structure

```bash
├── index.html
├── main.js
├── packages.json
├── README.md
├── static
│   └── {resource}.svg
├── style.css
├── tailwind.config.js
├── version.json
└── worker.js
```

### License

MIT OR Apache-2.0