use std::time::Duration;

use aegis_backend::{
    attendance::consume_challenge,
    models::StageKind,
    policy::{
        self, AttemptOutcome, AttemptPersistence, LocationInput, ReasonCode, SubmissionWindow,
    },
};
use axum::http::StatusCode;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use uuid::Uuid;

async fn isolated_pool() -> (SqlitePool, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("attendance-policy.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    (pool, directory)
}
async fn consume_once(
    pool: SqlitePool,
    user_id: Uuid,
    challenge_id: Uuid,
    stage_id: Uuid,
    received_at: i64,
) -> bool {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    match consume_challenge(
        &mut tx,
        user_id,
        challenge_id,
        "attendance",
        Some(stage_id),
        received_at,
    )
    .await
    {
        Ok(()) => {
            tx.commit().await.unwrap();
            true
        }
        Err(error) => {
            assert_eq!(error.status, StatusCode::CONFLICT);
            false
        }
    }
}

#[test]
fn submission_windows_enforce_exact_open_close_and_late_boundaries() {
    let window = |kind, received_at, closed_at, lesson_closed, lesson_ends_at| {
        policy::submission_window(
            kind,
            10,
            20,
            closed_at,
            0,
            lesson_ends_at,
            lesson_closed,
            received_at,
        )
    };

    assert_eq!(window(StageKind::CheckIn, 9, None, false, 1_000_000), None);
    assert_eq!(
        window(StageKind::CheckIn, 10, None, false, 1_000_000),
        Some(SubmissionWindow::Regular)
    );
    assert_eq!(
        window(StageKind::CheckIn, 19, None, false, 1_000_000),
        Some(SubmissionWindow::Regular)
    );
    assert_eq!(
        window(StageKind::CheckIn, 20, None, false, 1_000_000),
        Some(SubmissionWindow::Late)
    );
    assert_eq!(
        window(StageKind::CheckIn, 900_000, None, false, 1_000_000),
        Some(SubmissionWindow::Late)
    );
    assert_eq!(
        window(StageKind::CheckIn, 900_001, None, false, 1_000_000),
        None
    );
    assert_eq!(window(StageKind::CheckIn, 20, None, false, 20), None);
    assert_eq!(window(StageKind::CheckIn, 20, None, true, 1_000_000), None);
    assert_eq!(
        window(StageKind::CheckIn, 20, Some(21), false, 1_000_000),
        None
    );
    assert_eq!(
        window(StageKind::CheckIn, 21, Some(21), false, 1_000_000),
        Some(SubmissionWindow::Late)
    );
    assert_eq!(window(StageKind::Renew, 20, None, false, 1_000_000), None);
    assert_eq!(
        window(StageKind::CheckOut, 20, None, false, 1_000_000),
        None
    );
    assert!(policy::qr_token_valid_at(20, 19));
    assert!(!policy::qr_token_valid_at(20, 20));
}

#[test]
fn outcome_policy_preserves_the_review_exception_boundaries() {
    let regular = SubmissionWindow::Regular;
    let check_in = StageKind::CheckIn;
    assert_eq!(
        policy::attempt_outcome(check_in, regular, Some(true), true, true),
        AttemptOutcome::Passed
    );
    assert_eq!(
        policy::attempt_outcome(check_in, regular, Some(true), true, false),
        AttemptOutcome::Reviewable
    );
    assert_eq!(
        policy::attempt_outcome(check_in, regular, Some(true), false, true),
        AttemptOutcome::Reviewable
    );
    assert_eq!(
        policy::attempt_outcome(check_in, regular, Some(false), true, true),
        AttemptOutcome::Reviewable
    );
    assert_eq!(
        policy::attempt_outcome(check_in, regular, Some(false), true, false),
        AttemptOutcome::Failed
    );
    assert_eq!(
        policy::attempt_outcome(check_in, regular, Some(false), false, false),
        AttemptOutcome::Failed
    );
    assert_eq!(
        policy::attempt_outcome(check_in, regular, None, true, true),
        AttemptOutcome::Failed
    );

    assert_eq!(
        policy::attempt_outcome(check_in, SubmissionWindow::Late, None, true, true),
        AttemptOutcome::Reviewable
    );
    assert_eq!(
        policy::attempt_outcome(check_in, SubmissionWindow::Late, None, true, false),
        AttemptOutcome::Failed
    );
    for kind in [StageKind::Renew, StageKind::CheckOut] {
        assert_eq!(
            policy::attempt_outcome(kind, regular, None, true, true),
            AttemptOutcome::Passed
        );
        assert_eq!(
            policy::attempt_outcome(kind, regular, None, true, false),
            AttemptOutcome::Failed
        );
        assert_eq!(
            policy::attempt_outcome(kind, SubmissionWindow::Late, None, true, true),
            AttemptOutcome::Failed
        );
    }
}

#[test]
fn location_policy_distinguishes_missing_invalid_inaccurate_and_outside() {
    let target_latitude = 31.2304;
    let target_longitude = 121.4737;
    assert_eq!(
        policy::location_failure(None, target_latitude, target_longitude, 100.0),
        Some(ReasonCode::LocationMissing),
    );
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 90.001,
                longitude: 0.0,
                accuracy_m: 1.0
            }),
            0.0,
            0.0,
            100.0
        ),
        Some(ReasonCode::LocationInvalid),
    );
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 0.0,
                longitude: 0.0,
                accuracy_m: f64::INFINITY
            }),
            0.0,
            0.0,
            100.0
        ),
        Some(ReasonCode::LocationInaccurate),
    );
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 0.0,
                longitude: 0.0,
                accuracy_m: 0.0
            }),
            0.0,
            0.0,
            100.0
        ),
        Some(ReasonCode::LocationInaccurate),
    );
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 0.0,
                longitude: 0.0,
                accuracy_m: 100.0
            }),
            0.0,
            0.0,
            100.0
        ),
        None,
    );
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 0.0,
                longitude: 0.0,
                accuracy_m: 100.1
            }),
            0.0,
            0.0,
            100.0
        ),
        Some(ReasonCode::LocationInaccurate),
    );
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 0.0,
                longitude: 0.01,
                accuracy_m: 1.0
            }),
            0.0,
            0.0,
            100.0
        ),
        Some(ReasonCode::LocationOutside),
    );
    let boundary_distance =
        policy::haversine_meters(31.2304, 121.4740, target_latitude, target_longitude);
    assert_eq!(
        policy::location_failure(
            Some(LocationInput {
                latitude: 31.2304,
                longitude: 121.4740,
                accuracy_m: 3.0
            }),
            target_latitude,
            target_longitude,
            boundary_distance + 3.0,
        ),
        None,
    );
}

#[tokio::test]
async fn real_sql_result_transitions_are_reviewable_failed_monotonic_and_superseding() {
    let (pool, _directory) = isolated_pool().await;
    let teacher_id = Uuid::new_v4();
    let student_id = Uuid::new_v4();
    let course_id = Uuid::new_v4();
    let lesson_id = Uuid::new_v4();
    let stage_id = Uuid::new_v4();
    let stage_id_text = stage_id.to_string();
    let student_id_text = student_id.to_string();
    let created_at = 1_000_i64;
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, role, student_no, created_at) VALUES (?, ?, ?, ?, 'teacher', NULL, ?)")
        .bind(teacher_id.to_string()).bind("policy-teacher").bind("hash").bind("Policy teacher").bind(created_at)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, role, student_no, created_at) VALUES (?, ?, ?, ?, 'student', ?, ?)")
        .bind(student_id.to_string()).bind("policy-student").bind("hash").bind("Policy student").bind("policy-001").bind(created_at)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO courses (id, teacher_id, title, created_at) VALUES (?, ?, ?, ?)")
        .bind(course_id.to_string())
        .bind(teacher_id.to_string())
        .bind("Policy course")
        .bind(created_at)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO lessons (id, course_id, title, starts_at, ends_at, closed_at, created_at) VALUES (?, ?, ?, ?, ?, NULL, ?)")
        .bind(lesson_id.to_string()).bind(course_id.to_string()).bind("Policy lesson").bind(1_000_i64).bind(2_000_000_i64).bind(created_at)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO stages (id, lesson_id, ordinal, kind, opens_at, closes_at, closed_at, latitude, longitude, radius_m) VALUES (?, ?, 0, 'check_in', ?, ?, NULL, 0.0, 0.0, 100.0)")
        .bind(stage_id.to_string()).bind(lesson_id.to_string()).bind(1_000_i64).bind(2_000_i64)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO lesson_students (lesson_id, user_id) VALUES (?, ?)")
        .bind(lesson_id.to_string())
        .bind(student_id.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let challenge_id = Uuid::new_v4();
    sqlx::query("INSERT INTO face_challenges (id, user_id, stage_id, purpose, expires_at, consumed_at) VALUES (?, ?, ?, 'attendance', ?, NULL)")
        .bind(challenge_id.to_string()).bind(student_id.to_string()).bind(stage_id.to_string()).bind(5_000_i64)
        .execute(&pool).await.unwrap();
    let (first_consumer, second_consumer) = tokio::join!(
        consume_once(pool.clone(), student_id, challenge_id, stage_id, 2_000),
        consume_once(pool.clone(), student_id, challenge_id, stage_id, 2_000),
    );
    assert_ne!(first_consumer, second_consumer);
    let consumed_at: Option<i64> =
        sqlx::query_scalar("SELECT consumed_at FROM face_challenges WHERE id = ?")
            .bind(challenge_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(consumed_at, Some(2_000));
    assert!(!consume_once(pool.clone(), student_id, challenge_id, stage_id, 2_001).await);
    let expired_challenge_id = Uuid::new_v4();
    sqlx::query("INSERT INTO face_challenges (id, user_id, stage_id, purpose, expires_at, consumed_at) VALUES (?, ?, ?, 'attendance', ?, NULL)")
        .bind(expired_challenge_id.to_string()).bind(student_id.to_string()).bind(stage_id.to_string()).bind(2_000_i64)
        .execute(&pool).await.unwrap();
    assert!(
        !consume_once(
            pool.clone(),
            student_id,
            expired_challenge_id,
            stage_id,
            2_000
        )
        .await
    );
    let first_attempt = Uuid::new_v4();
    let review_id = Uuid::new_v4();
    let first_attempt_text = first_attempt.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    policy::apply_attempt_result(
        &mut tx,
        AttemptPersistence {
            attempt_id: &first_attempt_text,
            stage_id: &stage_id_text,
            user_id: &student_id_text,
            submitted_at: 1_500,
            qr_pass: Some(true),
            location_pass: true,
            face_pass: false,
            is_late: false,
            outcome: AttemptOutcome::Reviewable,
            reason_codes: &[ReasonCode::FaceSpoof],
        },
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO reviews (id, lesson_id, user_id, kind, attempt_id, reason, evidence_bytes, evidence_mime, status, reviewer_id, reviewed_at, decision_note, created_at) VALUES (?, ?, ?, 'partial', ?, 'reviewable evidence', NULL, NULL, 'pending', NULL, NULL, NULL, ?)")
        .bind(review_id.to_string()).bind(lesson_id.to_string()).bind(student_id.to_string()).bind(first_attempt.to_string()).bind(1_501_i64)
        .execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE stage_results SET status = 'pending', review_id = ? WHERE stage_id = ? AND user_id = ?")
        .bind(review_id.to_string()).bind(stage_id.to_string()).bind(student_id.to_string()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    let initial_result: (String, String, Option<String>) = sqlx::query_as("SELECT status, attempt_id, review_id FROM stage_results WHERE stage_id = ? AND user_id = ?")
        .bind(stage_id.to_string()).bind(student_id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        initial_result,
        (
            "pending".to_owned(),
            first_attempt.to_string(),
            Some(review_id.to_string())
        )
    );

    let failed_attempt = Uuid::new_v4();
    let failed_attempt_text = failed_attempt.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    policy::apply_attempt_result(
        &mut tx,
        AttemptPersistence {
            attempt_id: &failed_attempt_text,
            stage_id: &stage_id_text,
            user_id: &student_id_text,
            submitted_at: 1_600,
            qr_pass: Some(false),
            location_pass: false,
            face_pass: false,
            is_late: false,
            outcome: AttemptOutcome::Failed,
            reason_codes: &[ReasonCode::QrExpired, ReasonCode::LocationOutside],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let pending_after_failure: (String, String, Option<String>) = sqlx::query_as("SELECT status, attempt_id, review_id FROM stage_results WHERE stage_id = ? AND user_id = ?")
        .bind(stage_id.to_string()).bind(student_id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        pending_after_failure,
        (
            "pending".to_owned(),
            first_attempt.to_string(),
            Some(review_id.to_string())
        )
    );

    let passed_attempt = Uuid::new_v4();
    let passed_attempt_text = passed_attempt.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    policy::apply_attempt_result(
        &mut tx,
        AttemptPersistence {
            attempt_id: &passed_attempt_text,
            stage_id: &stage_id_text,
            user_id: &student_id_text,
            submitted_at: 1_700,
            qr_pass: Some(true),
            location_pass: true,
            face_pass: true,
            is_late: false,
            outcome: AttemptOutcome::Passed,
            reason_codes: &[],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let review: (String, String, i64, String) = sqlx::query_as(
        "SELECT status, reviewer_id, reviewed_at, decision_note FROM reviews WHERE id = ?",
    )
    .bind(review_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        review,
        (
            "rejected".to_owned(),
            student_id.to_string(),
            1_700,
            "superseded_by_success".to_owned()
        )
    );
    let passed_result: (String, String, Option<String>) = sqlx::query_as("SELECT status, attempt_id, review_id FROM stage_results WHERE stage_id = ? AND user_id = ?")
        .bind(stage_id.to_string()).bind(student_id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        passed_result,
        ("passed".to_owned(), passed_attempt.to_string(), None)
    );

    let late_failure = Uuid::new_v4();
    let late_failure_text = late_failure.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    policy::apply_attempt_result(
        &mut tx,
        AttemptPersistence {
            attempt_id: &late_failure_text,
            stage_id: &stage_id_text,
            user_id: &student_id_text,
            submitted_at: 1_800,
            qr_pass: Some(false),
            location_pass: false,
            face_pass: false,
            is_late: false,
            outcome: AttemptOutcome::Failed,
            reason_codes: &[ReasonCode::QrWrongStage],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let final_result: (String, String, Option<String>) = sqlx::query_as("SELECT status, attempt_id, review_id FROM stage_results WHERE stage_id = ? AND user_id = ?")
        .bind(stage_id.to_string()).bind(student_id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        final_result,
        ("passed".to_owned(), passed_attempt.to_string(), None)
    );
}
