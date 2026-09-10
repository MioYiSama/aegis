import { defineConfig } from "vite-plus";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react({ compiler: true })],
  run: {
    tasks: {
      "run:backend": {
        command: "cargo run -p aegis-backend --bin server",
      },
      "build:wasm": {
        command: [
          "cargo build -p aegis-wasm --profile aegis-wasm-release --target wasm32-unknown-unknown",
          "wasm-bindgen --out-dir src/wasm target/wasm32-unknown-unknown/aegis-wasm-release/aegis_wasm.wasm",
          "wasm-opt -Oz src/wasm/aegis_wasm_bg.wasm -o src/wasm/aegis_wasm_bg.wasm",
        ],
      },
      clean: {
        command: ["cargo clean", "rm -rf dist node_modules/.vite src/wasm"],
      },
    },
  },
});
