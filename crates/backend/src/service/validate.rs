use validator::ValidationError;

pub fn non_empty_after_trimmed(input: &str) -> Result<(), ValidationError> {
    if input.trim().is_empty() {
        return Err(ValidationError::new("不能为空"));
    }

    Ok(())
}
