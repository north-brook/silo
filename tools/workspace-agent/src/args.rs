pub(crate) fn required_flag_value<'a>(args: &'a [String], flag: &str) -> Result<&'a str, String> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("missing required flag: {flag}"))?;
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for flag: {flag}"))
}

pub(crate) fn optional_flag_value<'a>(
    args: &'a [String],
    flag: &str,
) -> Result<Option<&'a str>, String> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.get(index + 1)
        .map(String::as_str)
        .map(Some)
        .ok_or_else(|| format!("missing value for flag: {flag}"))
}
