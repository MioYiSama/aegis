import { expect, test } from "vite-plus/test";

import { signIn } from "@/api";

test("backend", async () => {
  const response = await signIn({ identity: "123456", password: "123456" });

  expect(response.status).toBe(200);

  console.log(response.headers);
});
