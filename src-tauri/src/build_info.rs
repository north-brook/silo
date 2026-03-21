pub(crate) const BUILD_FLAVOR: &str = env!("SILO_BUILD_FLAVOR");
const DEV_STATE_DIR_NAME: &str = ".silo-dev";
const PROD_STATE_DIR_NAME: &str = ".silo";
const PROD_FLAVOR: &str = "prod";

pub(crate) fn is_production_build() -> bool {
    BUILD_FLAVOR == PROD_FLAVOR
}

pub(crate) fn default_state_dir_name() -> &'static str {
    if is_production_build() {
        PROD_STATE_DIR_NAME
    } else {
        DEV_STATE_DIR_NAME
    }
}

pub(crate) fn updater_public_key() -> Option<&'static str> {
    option_env!("SILO_UPDATER_PUBLIC_KEY").filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_flavor_is_known() {
        assert!(matches!(BUILD_FLAVOR, "dev" | PROD_FLAVOR));
    }
}
