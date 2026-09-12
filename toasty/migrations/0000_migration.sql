CREATE TABLE "users" (
    "id" BLOB NOT NULL,
    "identity" TEXT NOT NULL,
    "password" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "role" INTEGER NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE UNIQUE INDEX "index_users_by_identity" ON "users" ("identity");
