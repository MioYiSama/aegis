import { defineConfig } from "orval";

export default defineConfig({
  petstore: {
    input: "./crates/backend/openapi.yaml",
    output: {
      clean: true,
      target: "./src/api/index.ts",
      mode: "split",
      client: "react-query",
    },
  },
});
