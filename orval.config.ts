import { defineConfig } from "orval";

export default defineConfig({
  petstore: {
    input: "./docs/openapi.json",
    output: {
      target: "./src/api/index.ts",
      mode: "tags-operations-split",
      client: "react-query",
      clean: true,
      baseUrl: {
        runtime: 'import.meta.env["VITE_BACKEND_URL"]!',
      },
    },
  },
});
