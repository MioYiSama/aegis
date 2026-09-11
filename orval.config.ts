import { defineConfig } from "orval";

export default defineConfig({
  petstore: {
    input: "./docs/openapi.json",
    output: {
      target: "./src/api/index.ts",
      mode: "tags-operations-split",
      client: "react-query",
      clean: true,
      baseUrl: "http://127.0.0.1:3000",
    },
  },
});
