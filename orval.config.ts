import { defineConfig } from "orval"
export default defineConfig({
  aegis: {
    input: "./src/api/openapi.json",
    output: {
      target: "./src/api/generated/client.ts",
      schemas: "./src/api/generated/models",
      client: "react-query",
      httpClient: "fetch",
      mode: "single",
      override: {
        mutator: { path: "./src/api/http.ts", name: "apiRequest" },
        formData: { path: "./src/api/form-data.ts", name: "toApiFormData" },
        fetch: {
          includeHttpResponseReturnType: false,
          forceSuccessResponse: true,
        },
      },
    },
  },
})
