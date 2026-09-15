CREATE TABLE "users" (
    "id" BLOB NOT NULL,
    "identity" TEXT NOT NULL,
    "password" TEXT NOT NULL,
    "name" TEXT,
    "role" INTEGER NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_users_by_identity" ON "users" ("identity");
-- #[toasty::breakpoint]
CREATE TABLE "sessions" (
    "id" BLOB NOT NULL,
    "token" TEXT NOT NULL,
    "expires_at" TEXT NOT NULL,
    "user_id" BLOB NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_sessions_by_token" ON "sessions" ("token");
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_sessions_by_user_id" ON "sessions" ("user_id");
