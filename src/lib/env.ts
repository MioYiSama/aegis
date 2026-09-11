import * as z from "zod/mini";

const env = z.parse(
  z.object({
    VITE_BACKEND_URL: z.url(),
  }),
  import.meta.env,
);

export default env;
