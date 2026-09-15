import { expect, test } from "vite-plus/test";

const BACKEND_URL = import.meta.env["VITE_BACKEND_URL"] ?? "http://localhost:3000";
const PASSWORD = "test-password";

async function post(path: string, body?: unknown, options?: RequestInit) {
  const headers = new Headers(options?.headers);
  if (body !== undefined) headers.set("Content-Type", "application/json");

  const response = await fetch(`${BACKEND_URL}${path}`, {
    ...options,
    method: "POST",
    headers,
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });

  return {
    data: await response.text(),
    headers: response.headers,
    status: response.status,
  };
}

function setCookie(headers: Headers, name: string) {
  const value = headers
    .get("set-cookie")
    ?.split(/,\s*(?=[^;,=\s]+=)/)
    .find((cookie) => cookie.startsWith(`${name}=`));

  expect(value, `响应应设置 ${name} Cookie`).toBeDefined();
  return value!;
}

function cookiePair(headers: Headers, name: string) {
  return setCookie(headers, name).split(";", 1)[0]!;
}

function authCookies(headers: Headers) {
  return ["access_token", "refresh_token"].map((name) => cookiePair(headers, name)).join("; ");
}

test("注册时拒绝无效账号信息", async () => {
  const suffix = crypto.randomUUID();

  const emptyIdentity = await post("/auth/sign-up", {
    identity: " ",
    password: PASSWORD,
    role: "Student",
  });
  expect(emptyIdentity.status).toBe(400);
  expect(emptyIdentity.data).toBe("学号/工号不能为空");

  const shortPassword = await post("/auth/sign-up", {
    identity: `short-password-${suffix}`,
    password: "12345",
    role: "Student",
  });
  expect(shortPassword.status).toBe(400);
  expect(shortPassword.data).toBe("密码长度必须至少为6个字符");

  const admin = await post("/auth/sign-up", {
    identity: `admin-${suffix}`,
    password: PASSWORD,
    role: "Admin",
  });
  expect(admin.status).toBe(400);
  expect(admin.data).toBe("禁止注册管理员身份的账号");
});

test("未登录时不能刷新会话", async () => {
  const response = await post("/auth/refresh");

  expect(response.status).toBe(401);
  expect(response.data).toBe("缺少刷新令牌");
});

test("完成注册、登录、刷新和退出的会话生命周期", async () => {
  const identity = `auth-test-${crypto.randomUUID()}`;

  const registered = await post("/auth/sign-up", {
    identity,
    name: "Auth Test",
    password: PASSWORD,
    role: "Student",
  });
  expect(registered.status).toBe(200);

  const accessCookie = setCookie(registered.headers, "access_token");
  expect(accessCookie).toContain("HttpOnly");
  expect(accessCookie).toContain("SameSite=Lax");
  expect(accessCookie).toContain("Path=/");

  const refreshCookie = setCookie(registered.headers, "refresh_token");
  expect(refreshCookie).toContain("HttpOnly");
  expect(refreshCookie).toContain("SameSite=Lax");
  expect(refreshCookie).toContain("Path=/auth");

  const duplicate = await post("/auth/sign-up", {
    identity,
    password: PASSWORD,
    role: "Student",
  });
  expect(duplicate.status).toBe(400);
  expect(duplicate.data).toBe("此学号/工号已被注册");

  const unknownAccount = await post("/auth/sign-in", {
    identity: `missing-${crypto.randomUUID()}`,
    password: PASSWORD,
  });
  expect(unknownAccount.status).toBe(401);
  expect(unknownAccount.data).toBe("账号或密码错误");

  const wrongPassword = await post("/auth/sign-in", {
    identity,
    password: "wrong-password",
  });
  expect(wrongPassword.status).toBe(401);
  expect(wrongPassword.data).toBe(unknownAccount.data);

  const signedIn = await post("/auth/sign-in", {
    identity,
    password: PASSWORD,
  });
  expect(signedIn.status).toBe(200);
  const signedInCookies = authCookies(signedIn.headers);
  const signedInRefreshToken = cookiePair(signedIn.headers, "refresh_token");

  const refreshed = await post("/auth/refresh", undefined, {
    headers: { Cookie: signedInCookies },
  });
  expect(refreshed.status).toBe(200);
  expect(cookiePair(refreshed.headers, "refresh_token")).not.toBe(signedInRefreshToken);
  const refreshedCookies = authCookies(refreshed.headers);

  const signedOut = await post("/auth/sign-out", undefined, {
    headers: { Cookie: refreshedCookies },
  });
  expect(signedOut.status).toBe(200);
  expect(setCookie(signedOut.headers, "access_token")).toContain("access_token=;");
  expect(setCookie(signedOut.headers, "refresh_token")).toContain("refresh_token=;");

  const refreshAfterSignOut = await post("/auth/refresh", undefined, {
    headers: { Cookie: refreshedCookies },
  });
  expect(refreshAfterSignOut.status).toBe(401);
  expect(refreshAfterSignOut.data).toBe("刷新令牌无效");
});
