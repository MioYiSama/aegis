import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite-plus";

export default defineConfig({
  plugins: [react({ compiler: true }), tailwindcss()],
  resolve: { tsconfigPaths: true },
  test: {
    include: ["tests/**/*.test.ts"],
  },
  lint: {
    ignorePatterns: ["src/wasm/**/*", "src/api/**/*", "src/components/ui/**/*"],
    options: { typeAware: true, typeCheck: true },
    plugins: ["react", "react-perf", "jsx-a11y"],
    jsPlugins: ["@shadcn/lint"],
  },
  fmt: {
    ignorePatterns: ["src/wasm/**/*", "src/api/**/*", "src/components/ui/**/*"],
    sortImports: true,
    sortPackageJson: true,
    sortTailwindcss: true,
  },
  run: {
    tasks: {
      backend: {
        command: "cargo run -p aegis-backend --bin server",
      },
      toasty: {
        command: "cargo run -p aegis-backend --bin toasty",
      },
      build: {
        dependsOn: ["wasm", "openapi"],
        command: ["vp build", "cargo build --release -p aegis-backend --bin server"],
      },
      openapi: {
        command: ["cargo run -p aegis-backend --bin openapi", "orval"],
      },
      wasm: {
        command: [
          "cargo build -p aegis-wasm --profile aegis-wasm-release --target wasm32-unknown-unknown",
          "wasm-bindgen --out-dir src/wasm target/wasm32-unknown-unknown/aegis-wasm-release/aegis_wasm.wasm",
          "wasm-opt -Oz src/wasm/aegis_wasm_bg.wasm -o src/wasm/aegis_wasm_bg.wasm",
        ],
      },
      clean: {
        command: [
          "cargo clean",
          "rm -rf dist src/api src/wasm",
          "rm -rf node_modules/.vite node_modules/.cache",
        ],
      },
    },
  },
});
