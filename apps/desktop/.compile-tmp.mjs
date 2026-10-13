import { compile } from "svelte/compiler";
import { readFileSync } from "fs";
const src = readFileSync(process.argv[2], "utf8");
const t = Date.now();
try { const r = compile(src, { filename: "page.svelte" }); console.log("ok", Date.now() - t, "ms", r.warnings.length, "warnings"); }
catch (e) { console.log("ERR", e.message); }
