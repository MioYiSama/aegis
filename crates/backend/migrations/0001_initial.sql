-- All persisted identifiers are application-generated UUID text values. All
-- timestamps are UTC Unix epoch milliseconds supplied by the application.
CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    username TEXT NOT NULL UNIQUE
        CHECK (length(username) BETWEEN 3 AND 64)
        CHECK (username = lower(trim(username)))
        CHECK (username NOT GLOB '*[^a-z0-9_.-]*'),
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL
        CHECK (length(trim(display_name)) BETWEEN 1 AND 128),
    role TEXT NOT NULL CHECK (role IN ('student', 'teacher')),
    student_no TEXT UNIQUE,
    created_at INTEGER NOT NULL CHECK (typeof(created_at) = 'integer'),
    CHECK (
        (role = 'student' AND student_no IS NOT NULL
            AND length(trim(student_no)) BETWEEN 1 AND 64)
        OR (role = 'teacher' AND student_no IS NULL)
    )
);

CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY NOT NULL CHECK (length(token_hash) = 32),
    user_id TEXT NOT NULL UNIQUE
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (typeof(created_at) = 'integer'),
    expires_at INTEGER NOT NULL CHECK (typeof(expires_at) = 'integer'),
    CHECK (expires_at > created_at)
);

CREATE TABLE face_templates (
    user_id TEXT PRIMARY KEY NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    embedding BLOB NOT NULL CHECK (length(embedding) = 512),
    model_id TEXT NOT NULL CHECK (length(model_id) > 0),
    created_at INTEGER NOT NULL CHECK (typeof(created_at) = 'integer')
);

CREATE TABLE courses (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    teacher_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 128),
    created_at INTEGER NOT NULL CHECK (typeof(created_at) = 'integer')
);
CREATE INDEX courses_teacher_created_idx ON courses(teacher_id, created_at, id);

CREATE TABLE enrollments (
    course_id TEXT NOT NULL
        REFERENCES courses(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    user_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    joined_at INTEGER NOT NULL CHECK (typeof(joined_at) = 'integer'),
    PRIMARY KEY (course_id, user_id)
);
CREATE INDEX enrollments_user_course_idx ON enrollments(user_id, course_id);

CREATE TABLE lessons (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    course_id TEXT NOT NULL
        REFERENCES courses(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 128),
    starts_at INTEGER NOT NULL CHECK (typeof(starts_at) = 'integer'),
    ends_at INTEGER NOT NULL CHECK (typeof(ends_at) = 'integer'),
    closed_at INTEGER CHECK (closed_at IS NULL OR typeof(closed_at) = 'integer'),
    created_at INTEGER NOT NULL CHECK (typeof(created_at) = 'integer'),
    CHECK (ends_at > starts_at)
);
CREATE INDEX lessons_course_starts_idx ON lessons(course_id, starts_at, id);

CREATE TABLE lesson_students (
    lesson_id TEXT NOT NULL
        REFERENCES lessons(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    user_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    PRIMARY KEY (lesson_id, user_id)
);
CREATE INDEX lesson_students_user_lesson_idx ON lesson_students(user_id, lesson_id);

CREATE TABLE stages (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    lesson_id TEXT NOT NULL
        REFERENCES lessons(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (typeof(ordinal) = 'integer' AND ordinal >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('check_in', 'renew', 'check_out')),
    opens_at INTEGER NOT NULL CHECK (typeof(opens_at) = 'integer'),
    closes_at INTEGER NOT NULL CHECK (typeof(closes_at) = 'integer'),
    closed_at INTEGER CHECK (closed_at IS NULL OR typeof(closed_at) = 'integer'),
    latitude REAL NOT NULL CHECK (latitude BETWEEN -90.0 AND 90.0),
    longitude REAL NOT NULL CHECK (longitude BETWEEN -180.0 AND 180.0),
    radius_m REAL NOT NULL CHECK (radius_m BETWEEN 1.0 AND 1000.0),
    UNIQUE (lesson_id, ordinal),
    CHECK (closes_at >= opens_at),
    CHECK (closed_at IS NULL OR closed_at BETWEEN opens_at AND closes_at)
);
CREATE UNIQUE INDEX stages_one_check_in_per_lesson_idx
    ON stages(lesson_id) WHERE kind = 'check_in';
CREATE UNIQUE INDEX stages_one_check_out_per_lesson_idx
    ON stages(lesson_id) WHERE kind = 'check_out';

CREATE TABLE qr_challenges (
    stage_id TEXT NOT NULL
        REFERENCES stages(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    slot INTEGER NOT NULL CHECK (typeof(slot) = 'integer' AND slot >= 0),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    token TEXT NOT NULL CHECK (length(token) = 32),
    expires_at INTEGER NOT NULL CHECK (typeof(expires_at) = 'integer'),
    PRIMARY KEY (stage_id, slot)
);
CREATE INDEX qr_challenges_expiry_idx ON qr_challenges(expires_at);

CREATE TABLE face_challenges (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    user_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    stage_id TEXT
        REFERENCES stages(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    purpose TEXT NOT NULL CHECK (purpose IN ('enroll', 'attendance')),
    expires_at INTEGER NOT NULL CHECK (typeof(expires_at) = 'integer'),
    consumed_at INTEGER CHECK (consumed_at IS NULL OR typeof(consumed_at) = 'integer'),
    CHECK (
        (purpose = 'enroll' AND stage_id IS NULL)
        OR (purpose = 'attendance' AND stage_id IS NOT NULL)
    )
);
CREATE INDEX face_challenges_user_expiry_idx ON face_challenges(user_id, expires_at);

CREATE TABLE attempts (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    stage_id TEXT NOT NULL
        REFERENCES stages(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    user_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    submitted_at INTEGER NOT NULL CHECK (typeof(submitted_at) = 'integer'),
    qr_pass INTEGER CHECK (qr_pass IS NULL OR qr_pass IN (0, 1)),
    location_pass INTEGER NOT NULL CHECK (location_pass IN (0, 1)),
    face_pass INTEGER NOT NULL CHECK (face_pass IN (0, 1)),
    is_late INTEGER NOT NULL CHECK (is_late IN (0, 1)),
    outcome TEXT NOT NULL CHECK (outcome IN ('passed', 'reviewable', 'failed')),
    reason_codes TEXT NOT NULL
);
CREATE INDEX attempts_stage_user_submitted_idx
    ON attempts(stage_id, user_id, submitted_at, id);
CREATE INDEX attempts_user_submitted_idx ON attempts(user_id, submitted_at, id);

CREATE TABLE reviews (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    lesson_id TEXT NOT NULL
        REFERENCES lessons(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    user_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('partial', 'late', 'leave')),
    attempt_id TEXT UNIQUE
        REFERENCES attempts(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (length(trim(reason)) BETWEEN 1 AND 1000),
    evidence_bytes BLOB,
    evidence_mime TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
    reviewer_id TEXT
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    reviewed_at INTEGER CHECK (reviewed_at IS NULL OR typeof(reviewed_at) = 'integer'),
    decision_note TEXT CHECK (decision_note IS NULL OR length(decision_note) <= 1000),
    created_at INTEGER NOT NULL CHECK (typeof(created_at) = 'integer'),
    CHECK (
        (kind IN ('partial', 'late') AND attempt_id IS NOT NULL)
        OR (kind = 'leave' AND attempt_id IS NULL)
    ),
    CHECK ((evidence_bytes IS NULL) = (evidence_mime IS NULL)),
    CHECK (
        evidence_mime IS NULL
        OR evidence_mime IN ('image/jpeg', 'image/png', 'application/pdf')
    ),
    CHECK (
        (status = 'pending' AND reviewer_id IS NULL AND reviewed_at IS NULL)
        OR (status IN ('approved', 'rejected') AND reviewer_id IS NOT NULL AND reviewed_at IS NOT NULL)
    )
);
CREATE INDEX reviews_lesson_created_idx ON reviews(lesson_id, created_at, id);
CREATE INDEX reviews_user_created_idx ON reviews(user_id, created_at, id);
CREATE INDEX reviews_status_lesson_idx ON reviews(status, lesson_id, user_id);
CREATE UNIQUE INDEX reviews_active_leave_per_student_idx
    ON reviews(lesson_id, user_id)
    WHERE kind = 'leave' AND status IN ('pending', 'approved');

CREATE TABLE stage_results (
    stage_id TEXT NOT NULL
        REFERENCES stages(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    user_id TEXT NOT NULL
        REFERENCES users(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('passed', 'pending', 'failed')),
    attempt_id TEXT NOT NULL
        REFERENCES attempts(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    review_id TEXT
        REFERENCES reviews(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    updated_at INTEGER NOT NULL CHECK (typeof(updated_at) = 'integer'),
    PRIMARY KEY (stage_id, user_id),
    CHECK (status <> 'pending' OR review_id IS NOT NULL)
);
CREATE INDEX stage_results_user_stage_idx ON stage_results(user_id, stage_id);
CREATE INDEX stage_results_review_idx ON stage_results(review_id) WHERE review_id IS NOT NULL;
