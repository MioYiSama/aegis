/* @ts-self-types="./aegis_wasm.d.ts" */
import * as wasm from "./aegis_wasm_bg.wasm";
import { __wbg_set_wasm } from "./aegis_wasm_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    greet
} from "./aegis_wasm_bg.js";
