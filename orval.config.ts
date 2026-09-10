import { defineConfig } from "orval";

export default defineConfig({
  petstore: {
    input: "./docs/openapi.json",
    output: {
      target: "./src/api/index.ts",
      mode: "split",
      client: "react-query",
      clean: true,
    },
  },
});
