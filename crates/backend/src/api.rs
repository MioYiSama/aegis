use axum::{
    Router,
    body::{Body, Bytes},
    extract::Extension,
    http::{HeaderValue, header},
    response::Response,
    routing::get,
};
use utoipa::{
    Modify, OpenApi,
    openapi::{
        Content, Ref, RefOr, Required,
        encoding::EncodingBuilder,
        path::{Operation, ParameterIn},
        response::ResponseBuilder,
        schema::{AdditionalProperties, Object, Schema},
        security::{ApiKey, ApiKeyValue, SecurityScheme},
    },
};

use crate::attendance;

#[derive(Debug, thiserror::Error)]
pub enum ApiSchemaError {
    #[error("OpenAPI serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("Generated OpenAPI document is missing required element: {0}")]
    Missing(&'static str),
}

struct SessionCookieSecurity;

impl Modify for SessionCookieSecurity {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                "aegis_session",
                "Opaque HttpOnly session cookie issued by POST /api/auth/login.",
            ))),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::auth::register,
        crate::auth::login,
        crate::auth::logout,
        crate::auth::me,
        crate::courses::list_courses,
        crate::courses::create_course,
        crate::courses::get_course,
        crate::courses::list_students,
        crate::courses::add_student,
        crate::courses::remove_student,
        crate::courses::create_lesson,
        crate::courses::list_lessons,
        crate::courses::get_lesson,
        crate::stages::create_stage,
        crate::stages::close_stage,
        crate::stages::close_lesson,
        crate::stages::get_stage_qr,
        crate::attendance::issue_face_challenge,
        crate::attendance::enroll_face,
        crate::attendance::submit_attempt,
        crate::reviews::create_attempt_review,
        crate::reviews::create_leave_review,
        crate::reviews::list_lesson_reviews,
        crate::reviews::get_review_evidence,
        crate::reviews::decide_review,
        crate::summary::get_attendance,
        crate::health,
        crate::ready,
        openapi_json
    ),
    components(schemas(crate::error::ApiErrorCode, attendance::AttemptPayload)),
    modifiers(&SessionCookieSecurity),
    tags(
        (name = "auth"),
        (name = "courses"),
        (name = "stages"),
        (name = "attendance"),
        (name = "reviews"),
        (name = "summary"),
        (name = "system")
    ),
    info(title = "Aegis Backend API", version = "0.1.0")
)]
struct ApiDocument;

#[derive(Clone)]
struct OpenApiBytes(Bytes);

/// Builds the document once during application startup and serves a shared immutable byte buffer.
pub fn router() -> Result<Router, ApiSchemaError> {
    let mut document = ApiDocument::openapi();
    finalize_document(&mut document)?;
    let serialized = Bytes::from(document.to_json()?);

    Ok(Router::new()
        .route("/api/openapi.json", get(openapi_json))
        .layer(Extension(OpenApiBytes(serialized))))
}

#[utoipa::path(
    get,
    path = "/api/openapi.json",
    operation_id = "api_get_openapi",
    responses((status = 200, description = "OpenAPI 3.1.0 document for the complete Aegis HTTP API", content_type = "application/json", body = serde_json::Value)),
    tag = "system"
)]
async fn openapi_json(Extension(OpenApiBytes(document)): Extension<OpenApiBytes>) -> Response {
    let mut response = Response::new(Body::from(document));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn finalize_document(document: &mut utoipa::openapi::OpenApi) -> Result<(), ApiSchemaError> {
    for schema in [
        "RegisterRequest",
        "LoginRequest",
        "CreateCourseRequest",
        "AddStudentRequest",
        "CreateLessonRequest",
        "CreateStageRequest",
        "FaceChallengeRequest",
        "AttemptPayload",
        "SubmittedLocation",
        "CreateReviewRequest",
        "DecisionRequest",
        "FaceEnrollmentMultipart",
        "AttendanceAttemptMultipart",
        "LeaveMultipart",
        "ApiError",
    ] {
        close_object_schema(document, schema)?;
    }

    set_string_constraints(
        document,
        "RegisterRequest",
        "username",
        3,
        64,
        Some("^[A-Za-z0-9_.-]{3,64}$"),
        "Normalized username: 3–64 ASCII letters, digits, dots, underscores, or hyphens.",
    )?;
    set_string_constraints(
        document,
        "RegisterRequest",
        "password",
        12,
        128,
        None,
        "Password length is 12–128 characters.",
    )?;
    set_string_constraints(
        document,
        "RegisterRequest",
        "display_name",
        1,
        128,
        None,
        "Trimmed display name; 1–128 characters.",
    )?;
    set_string_constraints(
        document,
        "RegisterRequest",
        "student_no",
        1,
        64,
        None,
        "Trimmed student number; required for students and absent or null for teachers.",
    )?;
    set_string_constraints(
        document,
        "LoginRequest",
        "username",
        3,
        64,
        Some("^[A-Za-z0-9_.-]{3,64}$"),
        "Registered username after trimming and ASCII lowercasing.",
    )?;
    set_string_constraints(
        document,
        "LoginRequest",
        "password",
        12,
        128,
        None,
        "Registered password length is 12–128 characters.",
    )?;
    set_string_constraints(
        document,
        "CreateCourseRequest",
        "title",
        1,
        128,
        None,
        "Trimmed, nonempty course title of at most 128 characters.",
    )?;
    set_string_constraints(
        document,
        "AddStudentRequest",
        "student_no",
        1,
        64,
        None,
        "Trimmed, nonempty student number of at most 64 characters.",
    )?;
    set_string_constraints(
        document,
        "CreateLessonRequest",
        "title",
        1,
        128,
        None,
        "Trimmed, nonempty lesson title of at most 128 characters.",
    )?;
    set_property_description(
        document,
        "CreateLessonRequest",
        "starts_at",
        "RFC3339 timestamp; lesson start time is fixed after creation.",
    )?;
    set_property_description(
        document,
        "CreateLessonRequest",
        "ends_at",
        "RFC3339 timestamp later than starts_at.",
    )?;
    set_property_description(
        document,
        "SubmittedLocation",
        "latitude",
        "Reported WGS-84 latitude; invalid coordinates fail the location factor.",
    )?;
    set_property_description(
        document,
        "SubmittedLocation",
        "longitude",
        "Reported WGS-84 longitude; invalid coordinates fail the location factor.",
    )?;
    set_property_description(
        document,
        "SubmittedLocation",
        "accuracy_m",
        "Reported positive location accuracy in meters; insufficient accuracy fails the location factor.",
    )?;

    let attempt_schema = component_object_mut(document, "AttendanceAttemptMultipart")?;
    let payload = attempt_schema
        .properties
        .get_mut("payload")
        .ok_or(ApiSchemaError::Missing(
            "AttendanceAttemptMultipart.payload",
        ))?;
    *payload = Ref::from_schema_name("AttemptPayload").into();

    set_multipart_encodings(
        document,
        "/api/face/enroll",
        &[
            ("frame_0", "image/jpeg, image/png"),
            ("frame_1", "image/jpeg, image/png"),
            ("frame_2", "image/jpeg, image/png"),
        ],
    )?;
    set_multipart_encodings(
        document,
        "/api/stages/{id}/attempts",
        &[
            ("payload", "application/json"),
            ("frame_0", "image/jpeg, image/png"),
            ("frame_1", "image/jpeg, image/png"),
            ("frame_2", "image/jpeg, image/png"),
            ("qr_image", "image/jpeg, image/png"),
        ],
    )?;
    set_multipart_encodings(
        document,
        "/api/lessons/{lesson_id}/leave",
        &[("evidence", "image/jpeg, image/png, application/pdf")],
    )?;

    set_pagination_constraints(document);
    add_too_large_responses(document);
    Ok(())
}

fn component_object_mut<'a>(
    document: &'a mut utoipa::openapi::OpenApi,
    name: &'static str,
) -> Result<&'a mut Object, ApiSchemaError> {
    let schema = document
        .components
        .as_mut()
        .and_then(|components| components.schemas.get_mut(name))
        .ok_or(ApiSchemaError::Missing(name))?;
    match schema {
        RefOr::T(Schema::Object(object)) => Ok(object),
        _ => Err(ApiSchemaError::Missing(name)),
    }
}

fn close_object_schema(
    document: &mut utoipa::openapi::OpenApi,
    name: &'static str,
) -> Result<(), ApiSchemaError> {
    component_object_mut(document, name)?.additional_properties =
        Some(Box::new(AdditionalProperties::FreeForm(false)));
    Ok(())
}

fn set_string_constraints(
    document: &mut utoipa::openapi::OpenApi,
    schema_name: &'static str,
    property_name: &'static str,
    minimum: usize,
    maximum: usize,
    pattern: Option<&'static str>,
    description: &'static str,
) -> Result<(), ApiSchemaError> {
    let property = component_object_mut(document, schema_name)?
        .properties
        .get_mut(property_name)
        .ok_or(ApiSchemaError::Missing(property_name))?;
    let RefOr::T(Schema::Object(property)) = property else {
        return Err(ApiSchemaError::Missing(property_name));
    };
    property.min_length = Some(minimum);
    property.max_length = Some(maximum);
    property.pattern = pattern.map(str::to_owned);
    property.description = Some(description.to_owned());
    Ok(())
}

fn set_property_description(
    document: &mut utoipa::openapi::OpenApi,
    schema_name: &'static str,
    property_name: &'static str,
    description: &'static str,
) -> Result<(), ApiSchemaError> {
    let property = component_object_mut(document, schema_name)?
        .properties
        .get_mut(property_name)
        .ok_or(ApiSchemaError::Missing(property_name))?;
    let RefOr::T(Schema::Object(property)) = property else {
        return Err(ApiSchemaError::Missing(property_name));
    };
    property.description = Some(description.to_owned());
    Ok(())
}

fn set_multipart_encodings(
    document: &mut utoipa::openapi::OpenApi,
    path: &'static str,
    fields: &[(&'static str, &'static str)],
) -> Result<(), ApiSchemaError> {
    let operation = document
        .paths
        .paths
        .get_mut(path)
        .and_then(|path| path.post.as_mut())
        .ok_or(ApiSchemaError::Missing(path))?;
    let content = operation
        .request_body
        .as_mut()
        .and_then(|body| body.content.get_mut("multipart/form-data"))
        .ok_or(ApiSchemaError::Missing("multipart/form-data request body"))?;
    for (field, content_type) in fields {
        content.encoding.insert(
            (*field).to_owned(),
            EncodingBuilder::new()
                .content_type(Some(*content_type))
                .build(),
        );
    }
    Ok(())
}

fn set_pagination_constraints(document: &mut utoipa::openapi::OpenApi) {
    for_each_operation(document, |operation| {
        let Some(parameters) = operation.parameters.as_mut() else {
            return;
        };
        for parameter in parameters.iter_mut().filter(|parameter| {
            parameter.parameter_in == ParameterIn::Query
                && (parameter.name == "limit" || parameter.name == "offset")
        }) {
            parameter.required = Required::False;
            let Some(RefOr::T(Schema::Object(schema))) = parameter.schema.as_mut() else {
                continue;
            };
            if parameter.name == "limit" {
                schema.minimum = Some(1_i64.into());
                schema.maximum = Some(100_i64.into());
                schema.default = Some(serde_json::json!(50));
                parameter.description = Some("Page size from 1 to 100; defaults to 50.".to_owned());
            } else {
                schema.minimum = Some(0_i64.into());
                schema.default = Some(serde_json::json!(0));
                parameter.description = Some("Zero-based offset; defaults to 0.".to_owned());
            }
        }
    });
}

fn add_too_large_responses(document: &mut utoipa::openapi::OpenApi) {
    for_each_operation(document, |operation| {
        if operation.request_body.is_none() || operation.responses.responses.contains_key("413") {
            return;
        }
        let response = ResponseBuilder::new()
            .description(
                "Request body exceeds its configured size limit; error code is invalid_input.",
            )
            .content(
                "application/json",
                Content::new(Some(Ref::from_schema_name("ApiError"))),
            )
            .build();
        operation
            .responses
            .responses
            .insert("413".to_owned(), response.into());
    });
}

fn for_each_operation(
    document: &mut utoipa::openapi::OpenApi,
    mut visit: impl FnMut(&mut Operation),
) {
    for item in document.paths.paths.values_mut() {
        if let Some(operation) = item.get.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.post.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.put.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.patch.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.delete.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.options.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.head.as_mut() {
            visit(operation);
        }
        if let Some(operation) = item.trace.as_mut() {
            visit(operation);
        }
    }
}
