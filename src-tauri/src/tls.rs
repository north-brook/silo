pub(crate) fn is_secure_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("https")
}

pub(crate) fn default_port_for_scheme(scheme: &str) -> u16 {
    if is_secure_scheme(scheme) {
        443
    } else {
        80
    }
}
